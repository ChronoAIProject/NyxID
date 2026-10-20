use super::*;
use crate::{
    models::user::{COLLECTION_NAME as USERS, UserType},
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};

fn body(key: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"idempotency_key":key,"text":text})).unwrap()
}

#[test]
fn invocation_canonicalization_is_owner_and_key_bound_and_input_is_closed() {
    crate::services::assistant_action_receipts::init_action_fingerprint_hmac_key(
        zeroize::Zeroizing::new([19; 32]),
    );
    let key = Uuid::new_v4().to_string();
    let a = body(&key, "hello");
    let b=serde_json::to_vec(&json!({"text":"hello","model":null,"idempotency_key":key,"agent_id":null,"conversation_id":null})).unwrap();
    let digest = parse(&a, "owner").unwrap().2;
    assert!(digest.starts_with("hmac-sha256:"));
    assert!(digest == parse(&b, "owner").unwrap().2);
    assert!(digest != parse(&a, "other").unwrap().2);
    assert!(
        digest
            != parse(&body(&Uuid::new_v4().to_string(), "hello"), "owner")
                .unwrap()
                .2
    );
    assert!(digest != parse(&body(&key, "goodbye"), "owner").unwrap().2);
    for value in [
        json!({"idempotency_key":key,"text":"hello","skip_destructive_confirmation":true}),
        json!({"idempotency_key":"e104bf09-0d68-4f2e-07aa-81e352cc3e66","text":"hello"}),
        json!({"idempotency_key":key,"text":""}),
        json!({"idempotency_key":key,"text":"x".repeat(32769)}),
        json!({"idempotency_key":key,"text":"hello","conversation_id":"nyxa-existing","agent_id":"specialist"}),
    ] {
        assert!(parse(&serde_json::to_vec(&value).unwrap(), "owner").is_err());
    }
}

#[tokio::test]
async fn invocation_concurrent_admission_replay_conflict_and_exact_cancel() {
    let db = connect_transaction_test_database("invocation_atomic").await;
    engine::ensure_indexes(&db).await.unwrap();
    invocations::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let key = Uuid::new_v4().to_string();
    let (_, start, digest) = parse(&body(&key, "hello"), &owner).unwrap();
    let (a, b) = tokio::join!(
        invocations::admit(&db, &owner, &key, &digest, &start, &state.encryption_keys),
        invocations::admit(&db, &owner, &key, &digest, &start, &state.encryption_keys)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert!(a.invocation.id == b.invocation.id);
    assert_ne!(a.claim.is_some(), b.claim.is_some());
    assert_eq!(
        db.collection::<mongodb::bson::Document>(crate::models::assistant_invocation::TURNS)
            .count_documents(mongodb::bson::doc! {})
            .await
            .unwrap(),
        1
    );
    assert!(
        invocations::admit(&db, &owner, &key, "changed", &start, &state.encryption_keys)
            .await
            .is_err()
    );
    let receipt = a.invocation;
    let claim = a.claim.or(b.claim).unwrap();
    invocations::cancel(&state, &owner, &key).await.unwrap();
    let snapshot = invocations::snapshot(&db, &owner, &key).await.unwrap();
    assert_eq!(snapshot.status, "cancelled");
    assert!(snapshot.terminal);
    let replay = invocations::admit(&db, &owner, &key, &digest, &start, &state.encryption_keys)
        .await
        .unwrap();
    assert!(replay.claim.is_none());
    assert!(replay.invocation.turn_id == receipt.turn_id);
    let (_, mut next, _) = parse(&body(&Uuid::new_v4().to_string(), "next"), &owner).unwrap();
    next.conversation_id = Some(claim.conversation.id.clone());
    let successor = engine::begin_turn(&db, &owner, &next, &state.encryption_keys)
        .await
        .unwrap();
    invocations::cancel(&state, &owner, &key).await.unwrap();
    let current = engine::get(&db, &owner, &successor.id).await.unwrap();
    assert!(!current.active_turn.unwrap().stop_requested);
    assert!(invocations::snapshot(&db, "other", &key).await.is_err());
}

async fn admitted(name: &str) -> (AppState, String, invocations::Admission) {
    let db = connect_transaction_test_database(name).await;
    engine::ensure_indexes(&db).await.unwrap();
    invocations::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let key = Uuid::new_v4().to_string();
    let (_, start, digest) = parse(&body(&key, "bounded work"), &owner).unwrap();
    let admission = invocations::admit(&db, &owner, &key, &digest, &start, &state.encryption_keys)
        .await
        .unwrap();
    (state, owner, admission)
}

#[tokio::test]
async fn invocation_old_credential_cannot_acquire_successor_authority() {
    use crate::services::assistant_acknowledgement_service as acks;
    let (state, owner, admission) = admitted("invocation_key_generation").await;
    let key = admission.invocation.idempotency_key;
    let claim = admission.claim.unwrap();
    let old_key = claim.credential.api_key_id.clone();
    let old_hash = crate::crypto::token::hash_token(&claim.credential.raw_key);
    let chat = acks::for_key(&state.db, &owner, Some(&old_key))
        .await
        .unwrap()
        .unwrap();
    assert!(chat.invocation_controlled);
    chat.turn_fence().require_live().unwrap();
    invocations::cancel(&state, &owner, &key).await.unwrap();
    let stopped = acks::for_key(&state.db, &owner, Some(&old_key))
        .await
        .unwrap()
        .unwrap();
    assert!(stopped.turn_fence().require_live().is_err());
    // The ordinary live-turn rollout is off; invocation fencing is mandatory.
    assert!(
        acks::enforce_turn_gate(
            &state.db,
            &stopped.turn_fence(),
            &owner,
            Some(&old_key),
            None,
            None,
            None,
            "/mcp"
        )
        .await
        .is_err()
    );
    let (_, mut start, _) = parse(&body(&Uuid::new_v4().to_string(), "successor"), &owner).unwrap();
    start.conversation_id = Some(claim.conversation.id);
    let successor = engine::begin_turn(&state.db, &owner, &start, &state.encryption_keys)
        .await
        .unwrap();
    assert_ne!(successor.credential_api_key_id, old_key);
    assert!(
        state
            .db
            .collection::<mongodb::bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .find_one(mongodb::bson::doc! {"key_hash":old_hash,"is_active":true})
            .await
            .unwrap()
            .is_none()
    );
    let current = acks::for_key(&state.db, &owner, Some(&successor.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    current.turn_fence().require_live().unwrap();
    assert!(current.invocation_id.is_none());
    assert!(
        invocations::revalidate_after_approval(&state.db, &chat)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn invocation_late_events_are_fenced_and_async_results_are_individual() {
    use crate::models::assistant_conversation::TurnOrigin;
    use crate::services::{assistant_team_service as team, async_service_operation as operations};
    let (state, owner, admission) = admitted("invocation_events").await;
    let claim = admission.claim.unwrap();
    let root = &admission.invocation.id;
    let ordinary = team::event("notice", "ordinary".into(), None);
    let first =
        team::event(operations::EVENT_KIND, "first".into(), None).for_invocation(Some(root));
    let second =
        team::event(operations::EVENT_KIND, "second".into(), None).for_invocation(Some(root));
    let same = team::event("notice", "same".into(), None).for_invocation(Some(root));
    let other = team::event("notice", "other".into(), None).for_invocation(Some("other"));
    let mut row = claim.conversation.clone();
    row.pending_events = vec![
        ordinary.clone(),
        first.clone(),
        same.clone(),
        second.clone(),
        other.clone(),
    ];
    let selected = operations::select_events(&mut row, TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, first.id);
    let selected = operations::select_events(&mut row, TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, same.id);
    let selected = operations::select_events(&mut row, TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, second.id);
    assert!(row.pending_events.iter().any(|e| e.id == ordinary.id));
    assert!(row.pending_events.iter().any(|e| e.id == other.id));
    invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    invocations::push_events(&state.db, &owner, &row.id, vec![first, ordinary.clone()])
        .await
        .unwrap();
    let row = engine::get(&state.db, &owner, &row.id).await.unwrap();
    assert!(row.pending_events.iter().all(|e| e.invocation_id.is_none()));
    assert!(row.pending_events.iter().any(|e| e.id == ordinary.id));
}

#[tokio::test]
async fn invocation_and_source_queues_preserve_work_and_rollback_overflow() {
    use crate::models::assistant_conversation::{COLLECTION_NAME as CONVERSATIONS, TurnOrigin};
    use crate::services::{assistant_team_service as team, async_service_operation as operations};
    use mongodb::bson::{self, Document, doc};
    let (state, owner, admission) = admitted("invocation_source_queue").await;
    let conversation = admission.claim.unwrap().conversation.id;
    let tracked = team::event("notice", "invocation continuation".into(), None)
        .for_invocation(Some(&admission.invocation.id));
    let mut source = team::event("message", "channel question".into(), None);
    source.source_event_ids = vec![Uuid::new_v4().to_string()];
    let mut events = vec![tracked.clone(), source.clone()];
    events.extend(
        (2..engine::MAX_PENDING_EVENTS).map(|_| team::event("notice", "update".into(), None)),
    );
    let rows = state.db.collection::<Document>(CONVERSATIONS);
    rows.update_one(
        doc! {"_id": &conversation},
        doc! {"$set": {"pending_events": bson::to_bson(&events).unwrap()}},
    )
    .await
    .unwrap();
    for incoming in [
        team::event("notice", "ordinary".into(), None),
        team::event("notice", "tracked".into(), None)
            .for_invocation(Some(&admission.invocation.id)),
    ] {
        assert!(matches!(
            engine::push_events(&state.db, &owner, &conversation, vec![incoming]).await,
            Err(AppError::Conflict(_))
        ));
        let current = engine::get(&state.db, &owner, &conversation).await.unwrap();
        assert_eq!(
            bson::to_bson(&current.pending_events).unwrap(),
            bson::to_bson(&events).unwrap()
        );
    }
    // Exact duplicate delivery is harmless even at capacity; changed lineage is refused.
    engine::push_events(&state.db, &owner, &conversation, vec![tracked.clone()])
        .await
        .unwrap();
    let mut changed = tracked.clone();
    changed.invocation_id = None;
    assert!(
        engine::push_events(&state.db, &owner, &conversation, vec![changed])
            .await
            .is_err()
    );
    events.pop();
    rows.update_one(
        doc! {"_id": &conversation},
        doc! {"$set": {"pending_events": bson::to_bson(&events).unwrap()}},
    )
    .await
    .unwrap();
    assert!(
        engine::push_events(
            &state.db,
            &owner,
            &conversation,
            vec![
                team::event("notice", "first".into(), None),
                team::event("notice", "overflow".into(), None)
            ]
        )
        .await
        .is_err()
    );
    let mut current = engine::get(&state.db, &owner, &conversation).await.unwrap();
    assert_eq!(
        bson::to_bson(&current.pending_events).unwrap(),
        bson::to_bson(&events).unwrap()
    );
    let ordinary = operations::select_events(&mut current, TurnOrigin::User);
    assert!(
        ordinary
            .iter()
            .all(|e| e.invocation_id.is_none() && e.source_event_ids.is_empty())
    );
    assert_eq!(current.pending_events.len(), 2);
    assert_eq!(
        operations::select_events(&mut current, TurnOrigin::Event)[0].id,
        tracked.id
    );
    assert_eq!(
        operations::select_events(&mut current, TurnOrigin::Event)[0].id,
        source.id
    );
    assert!(current.pending_events.is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn invocation_deleted_root_recovers_post_commit_cancellation_and_keeps_tombstone() {
    let (state, owner, admission) = admitted("invocation_delete_recovery").await;
    let receipt = admission.invocation;
    let claim = admission.claim.unwrap();
    engine::delete(&state.db, &owner, &claim.conversation.id)
        .await
        .unwrap();
    let before = invocations::get(&state.db, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert!(before.cancel_requested && before.conversation_deleted);
    // Simulate process loss before the post-delete cleanup call.
    invocations::reconcile_cancellations(&state).await.unwrap();
    let snapshot = invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert_eq!(snapshot.status, "deleted");
    assert!(snapshot.terminal);
    let (_, start, _) = parse(&body(&receipt.idempotency_key, "bounded work"), &owner).unwrap();
    let replay = invocations::admit(
        &state.db,
        &owner,
        &receipt.idempotency_key,
        &receipt.request_digest,
        &start,
        &state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(replay.claim.is_none());
    assert_eq!(replay.invocation.turn_id, receipt.turn_id);
}

#[tokio::test]
async fn invocation_last_turn_can_complete_but_blocked_continuation_requires_review() {
    use crate::models::assistant_invocation::{COLLECTION_NAME as ROOTS, TURNS};
    use crate::services::assistant_team_service as team;
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_turn_limit").await;
    let receipt = admission.invocation;
    let claim = admission.claim.unwrap();
    state
        .db
        .collection::<Document>(ROOTS)
        .update_one(
            doc! {"_id":&receipt.id},
            doc! {"$set":{"turn_count":invocations::MAX_TURNS}},
        )
        .await
        .unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        invocations::fence_in_session(&state.db, &mut session, &owner, Some(&receipt.id))
            .await
            .unwrap()
    );
    session.commit_transaction().await.unwrap();
    state
        .db
        .collection::<Document>(TURNS)
        .update_one(
            doc! {"_id":&receipt.turn_id},
            doc! {"$set":{"state":"completed","hooks_pending":false}},
        )
        .await
        .unwrap();
    let event =
        team::event("notice", "continuation".into(), None).for_invocation(Some(&receipt.id));
    invocations::push_events(&state.db, &owner, &claim.conversation.id, vec![event])
        .await
        .unwrap();
    let snapshot = invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert_eq!(snapshot.status, "requires_review");
    assert!(!snapshot.terminal);
    assert_eq!(snapshot.error_code.as_deref(), Some("turn_limit"));
    // A last turn with no continuation can still durably complete.
    state
        .db
        .collection::<Document>(ROOTS)
        .update_one(
            doc! {"_id":&receipt.id},
            doc! {"$set":{"continuation_blocked":false}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&claim.conversation.id},
            doc! {"$set":{"pending_events":[]}},
        )
        .await
        .unwrap();
    assert_eq!(
        invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
            .await
            .unwrap()
            .status,
        "completed"
    );
}

#[tokio::test]
async fn invocation_delayed_skill_outbox_and_expired_decision_are_not_completion() {
    use crate::models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS, assistant_invocation::TURNS,
    };
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_pending_producers").await;
    let receipt = admission.invocation;
    state
        .db
        .collection::<Document>(TURNS)
        .update_one(
            doc! {"_id":&receipt.turn_id},
            doc! {"$set":{"state":"completed","hooks_pending":false}},
        )
        .await
        .unwrap();
    let card = Uuid::new_v4().to_string();
    state.db.collection::<Document>(ACKS).insert_one(doc!{
        "_id":&card,"user_id":&owner,"conversation_id":&receipt.conversation_id,
        "invocation_id":&receipt.id,"status":"allowed","kind":"skills","skill_notification_pending":true,
        "expires_at":mongodb::bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1)),
    }).await.unwrap();
    let snapshot = invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert_eq!(snapshot.status, "waiting");
    assert!(!snapshot.terminal);
    state
        .db
        .collection::<Document>(ACKS)
        .update_one(
            doc! {"_id":&card},
            doc! {"$set":{"status":"expired","decider":"user","skill_notification_pending":false}},
        )
        .await
        .unwrap();
    let snapshot = invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert_eq!(snapshot.status, "requires_review");
    assert_eq!(
        snapshot.error_code.as_deref(),
        Some("owner_decision_expired")
    );
    state
        .db
        .collection::<Document>(ACKS)
        .update_one(
            doc! {"_id":&card},
            doc! {"$set":{"skill_notification_pending":true}},
        )
        .await
        .unwrap();
    invocations::cancel(&state, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert_eq!(
        invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
            .await
            .unwrap()
            .status,
        "cancelled"
    );
    let card = state
        .db
        .collection::<Document>(ACKS)
        .find_one(doc! {"_id":&card})
        .await
        .unwrap()
        .unwrap();
    assert!(!card.contains_key("skill_notification_pending"));
}

#[tokio::test]
async fn invocation_generation_switch_preserves_unrelated_pending_work() {
    use crate::models::assistant_acknowledgement::COLLECTION_NAME as ACKS;
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_generation_pending").await;
    let claim = admission.claim.unwrap();
    invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    let (_, mut start, _) = parse(&body(&Uuid::new_v4().to_string(), "next"), &owner).unwrap();
    start.conversation_id = Some(claim.conversation.id.clone());
    for (collection, pending) in [
        (
            ACKS,
            doc! {"status":"pending","expires_at":mongodb::bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))},
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"state":"waiting"},
        ),
    ] {
        let mut pending = pending;
        pending.insert("_id", "pending-fixture");
        pending.insert("user_id", &owner);
        pending.insert("conversation_id", &claim.conversation.id);
        state
            .db
            .collection::<Document>(collection)
            .insert_one(pending)
            .await
            .unwrap();
        assert!(matches!(
            engine::begin_turn(&state.db, &owner, &start, &state.encryption_keys).await,
            Err(AppError::Conflict(_))
        ));
        let current = engine::get(&state.db, &owner, &claim.conversation.id)
            .await
            .unwrap();
        assert_eq!(current.credential_api_key_id, claim.credential.api_key_id);
        assert!(current.active_turn.is_none());
        state
            .db
            .collection::<Document>(collection)
            .delete_one(doc! {"_id":"pending-fixture"})
            .await
            .unwrap();
    }
    let successor = engine::begin_turn(&state.db, &owner, &start, &state.encryption_keys)
        .await
        .unwrap();
    assert_ne!(successor.credential_api_key_id, claim.credential.api_key_id);
}

#[tokio::test]
async fn invocation_foreign_agent_rejection_does_not_provision_default() {
    use mongodb::bson::{Document, doc};
    let db = connect_transaction_test_database("invocation_foreign_agent").await;
    let owner = Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let key = Uuid::new_v4().to_string();
    let (_, mut start, digest) = parse(&body(&key, "request"), &owner).unwrap();
    start.agent_id = Some(Uuid::new_v4().to_string());
    assert!(
        invocations::admit(&db, &owner, &key, &digest, &start, &state.encryption_keys)
            .await
            .is_err()
    );
    for collection in [
        crate::models::assistant_agent::COLLECTION_NAME,
        crate::models::assistant_invocation::COLLECTION_NAME,
        crate::models::assistant_conversation::COLLECTION_NAME,
        crate::models::assistant_agent_credential::COLLECTION_NAME,
    ] {
        assert_eq!(
            db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn invocation_direct_hidden_conversation_id_is_rejected_without_admission() {
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_hidden_context").await;
    let claim = admission.claim.unwrap();
    invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    let (_, mut start, digest) =
        parse(&body(&Uuid::new_v4().to_string(), "hidden"), &owner).unwrap();
    start.conversation_id = Some(claim.conversation.id.clone());
    let roots = state
        .db
        .collection::<Document>(crate::models::assistant_invocation::COLLECTION_NAME);
    let count = roots.count_documents(doc! {}).await.unwrap();
    for (field, value) in [
        ("group_id", mongodb::bson::Bson::String("group".into())),
        ("automation_thread", mongodb::bson::Bson::Boolean(true)),
        (
            "channel",
            mongodb::bson::Bson::Document(doc! {
                "nyxbot_channel_id":"adapter-binding", "partition":"external-session",
                "platform":"telnyx_relay", "thread":bson::Bson::Null,
            }),
        ),
        (
            "voice_parent_conversation_id",
            mongodb::bson::Bson::String("voice".into()),
        ),
    ] {
        let fields = doc! {field:value};
        state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(doc! {"_id":&claim.conversation.id}, doc! {"$set":fields})
            .await
            .unwrap();
        let key = Uuid::new_v4().to_string();
        assert!(matches!(
            invocations::admit(
                &state.db,
                &owner,
                &key,
                &digest,
                &start,
                &state.encryption_keys
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
        assert_eq!(roots.count_documents(doc! {}).await.unwrap(), count);
        state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&claim.conversation.id},
                doc! {"$unset":{field:""}},
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn invocation_deleted_root_immediately_denies_child_execution_and_card_consumption() {
    use crate::services::assistant_acknowledgement_service as acks;
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_deleted_child_fence").await;
    let receipt = admission.invocation;
    let root = admission.claim.unwrap();
    let (_, mut start, _) =
        parse(&body(&Uuid::new_v4().to_string(), "descendant"), &owner).unwrap();
    start.invocation_id = Some(receipt.id.clone());
    let child = engine::begin_turn(&state.db, &owner, &start, &state.encryption_keys)
        .await
        .unwrap();
    let chat = acks::for_key(&state.db, &owner, Some(&child.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let card = Uuid::new_v4().to_string();
    let args = json!({"resource":"safe-fixture"});
    state.db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME).insert_one(doc!{
        "_id":&card,"user_id":&owner,"conversation_id":&child.id,"api_key_id":&chat.api_key_id,
        "invocation_id":&receipt.id,"status":"allowed","kind":"action","tool_name":"fixture",
        "arguments_digest":acks::arguments_digest(&args),"decided_by":"user",
        "expires_at":mongodb::bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1)),
    }).await.unwrap();
    engine::delete(&state.db, &owner, &root.conversation.id)
        .await
        .unwrap();
    // Before any post-delete/sweep node cleanup, root cancellation is decisive.
    assert!(
        acks::enforce_turn_gate(
            &state.db,
            &chat.turn_fence(),
            &owner,
            Some(&chat.api_key_id),
            None,
            None,
            None,
            "/mcp"
        )
        .await
        .is_err()
    );
    assert!(
        acks::consume_action(&state.db, &chat, &card, "fixture", &args)
            .await
            .is_err()
    );
    let unchanged = state
        .db
        .collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":&card})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.get_str("status").unwrap(), "allowed");
    invocations::reconcile_cancellations(&state).await.unwrap();
    assert!(
        engine::get(&state.db, &owner, &child.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    assert_eq!(
        invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
            .await
            .unwrap()
            .status,
        "deleted"
    );
}

#[tokio::test]
async fn invocation_async_cancellation_attempt_is_durable_and_never_replayed() {
    use crate::models::async_service_operation::{AsyncServiceOperation, COLLECTION_NAME as OPS};
    use crate::services::async_service_operation as operations;
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_async_cancel_receipt").await;
    let receipt = admission.invocation;
    invocations::cancel(&state, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    let now = mongodb::bson::DateTime::now();
    let watch:AsyncServiceOperation=mongodb::bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":&owner,"conversation_id":&receipt.conversation_id,
        "agent_id":"fixture","turn_id":&receipt.turn_id,"api_key_id":"fixture","service_id":"fixture","submit_endpoint_id":"fixture",
        "contract":{"status_operation":"status","result_operation":"result","cancel_operation":"cancel","id_field":"/id","id_parameter":"id","status_field":"/status","success_states":["done"],"failure_states":["failed"],"error_field":null},
        "delivery":{"invocation_id":&receipt.id,"origin":"user","channel":null,"reply_channel":null,"group_id":null,"group_request_id":null,"voice_request_id":null,"trigger_run_id":null,"report_to":null,"report_provenance":0},
        "operation_id":"fixture","state":"cancelling","reason":null,"attempts":0,"lease_id":"fixture","lease_until":null,
        "created_at":now,"deadline":now,"poll_at":now,"expires_at":now,"result_encrypted":null
    }).unwrap();
    state
        .db
        .collection::<AsyncServiceOperation>(OPS)
        .insert_one(&watch)
        .await
        .unwrap();
    assert!(
        operations::begin_tracked_cancellation(&state.db, &watch)
            .await
            .unwrap()
    );
    assert!(
        !operations::begin_tracked_cancellation(&state.db, &watch)
            .await
            .unwrap()
    );
    let snapshot = invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert_eq!(snapshot.status, "requires_review");
    assert_eq!(
        snapshot.error_code.as_deref(),
        Some("async_cancellation_unconfirmed")
    );
    operations::finish_tracked_cancellation(&state.db, &watch, false, "cancellation_unknown")
        .await
        .unwrap();
    // Removing the watch cannot erase the root's durable uncertain outcome.
    state
        .db
        .collection::<Document>(OPS)
        .delete_one(doc! {"_id":&watch.id})
        .await
        .unwrap();
    assert_eq!(
        invocations::snapshot(&state.db, &owner, &receipt.idempotency_key)
            .await
            .unwrap()
            .status,
        "requires_review"
    );
    let mut success = watch.clone();
    success.id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<AsyncServiceOperation>(OPS)
        .insert_one(&success)
        .await
        .unwrap();
    assert!(
        operations::begin_tracked_cancellation(&state.db, &success)
            .await
            .unwrap()
    );
    operations::finish_tracked_cancellation(&state.db, &success, true, "cancelled")
        .await
        .unwrap();
    let root = invocations::get(&state.db, &owner, &receipt.idempotency_key)
        .await
        .unwrap();
    assert_eq!(root.unconfirmed_async_cancellations, vec![watch.id]);
}

struct DelayedInvocationEgress(tokio::sync::Notify);
#[async_trait::async_trait]
impl crate::services::platform_manager_contract::NyxAgentEgress for DelayedInvocationEgress {
    async fn request(
        &self,
        _: &crate::services::platform_manager_contract::NyxAgentExecutionContext,
        _: &crate::services::platform_manager_contract::RuntimeCredential,
        _: crate::services::platform_manager_contract::NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        self.0.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn invocation_delayed_cancelled_runtime_does_not_stop_successor_machine_job() {
    use crate::services::assistant_nyxagent_runtime as runtime;
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_runtime_successor").await;
    let claim = admission.claim.unwrap();
    let conversation = claim.conversation.id.clone();
    invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    let (_, mut start, _) = parse(&body(&Uuid::new_v4().to_string(), "successor"), &owner).unwrap();
    start.conversation_id = Some(conversation.clone());
    let successor = engine::begin_turn(&state.db, &owner, &start, &state.encryption_keys)
        .await
        .unwrap();
    let turn = successor.active_turn.as_ref().unwrap().turn_id.clone();
    let job = Uuid::new_v4().to_string();
    state
        .db
        .collection::<Document>(crate::models::machine_job::COLLECTION_NAME)
        .insert_one(doc! {
            "_id":&job,"user_id":&owner,"node_id":"fixture","conversation_id":&conversation,
            "issuing_turn_id":&turn,"state":"running","created_at":mongodb::bson::DateTime::now(),
            "expires_at":mongodb::bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))
        })
        .await
        .unwrap();
    // Start the old worker only after successor authority and its job exist.
    // This models a worker delayed in scheduling, with no 250ms race window.
    let egress = std::sync::Arc::new(DelayedInvocationEgress(tokio::sync::Notify::new()));
    let permit = state.direct_chat_limiter.try_acquire(&owner).await.unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(32);
    let worker = tokio::spawn(runtime::run_owner_turn(
        state.clone(),
        egress.clone(),
        claim.conversation,
        "fixture".into(),
        claim.credential,
        None,
        permit,
        runtime::Events {
            sender,
            cursor: 0,
            reset_notified: false,
        },
    ));
    let settled = tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        settled.error.as_ref().map(|error| error.code),
        Some("cancelled")
    );
    let current = engine::get(&state.db, &owner, &conversation).await.unwrap();
    let active = current.active_turn.unwrap();
    assert_eq!(active.turn_id, turn);
    assert!(!active.stop_requested);
    let job = state
        .db
        .collection::<Document>(crate::models::machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":job})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.get_str("state").unwrap(), "running");
    // Prove this fixture detects the old broad fallback, without rebuilding
    // production code with a deliberately reintroduced cancellation bug.
    crate::services::machine_service::cancel_conversation(&state, &owner, &conversation)
        .await
        .unwrap();
    assert!(
        engine::get(&state.db, &owner, &conversation)
            .await
            .unwrap()
            .active_turn
            .unwrap()
            .stop_requested
    );
    let after = state
        .db
        .collection::<Document>(crate::models::machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":job.get_str("_id").unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.get_str("state").unwrap(), "finished");
}

#[tokio::test]
async fn invocation_cards_dedup_and_consume_are_scoped_to_origin_family() {
    use crate::models::assistant_acknowledgement::COLLECTION_NAME as ACKS;
    use crate::services::assistant_acknowledgement_service as acks;
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_card_families").await;
    let claim = admission.claim.unwrap();
    let chat = acks::for_key(&state.db, &owner, Some(&claim.credential.api_key_id))
        .await
        .unwrap()
        .unwrap();
    let args = json!({"fixture":"same"});
    let request = || acks::Request {
        kind: "action",
        service: None,
        tool: Some("fixture"),
        arguments: Some(&args),
        summary: "fixture",
        platform: false,
    };
    let first = acks::request(&state.db, &chat, request()).await.unwrap();
    let replay = acks::request(&state.db, &chat, request()).await.unwrap();
    assert_eq!(first.id, replay.id);
    // Keep the same live key/turn so this exercises card predicates, rather
    // than getting a refusal from credential rotation or live-turn validation.
    let mut browser = chat.clone();
    browser.invocation_id = None;
    let browser_card = acks::request(&state.db, &browser, request()).await.unwrap();
    assert_ne!(first.id, browser_card.id);
    let mut other_root = admission.invocation.clone();
    other_root.id = Uuid::new_v4().to_string();
    other_root.idempotency_key = Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::assistant_invocation::AssistantInvocation>(
            crate::models::assistant_invocation::COLLECTION_NAME,
        )
        .insert_one(&other_root)
        .await
        .unwrap();
    let mut other = chat.clone();
    other.invocation_id = Some(other_root.id);
    let other_card = acks::request(&state.db, &other, request()).await.unwrap();
    assert_ne!(first.id, other_card.id);
    state
        .db
        .collection::<Document>(ACKS)
        .update_one(
            doc! {"_id":&first.id},
            doc! {"$set":{"status":"allowed","decided_by":"user","decider":"user"}},
        )
        .await
        .unwrap();
    assert!(
        !acks::consume_action(&state.db, &browser, &first.id, "fixture", &args)
            .await
            .unwrap()
    );
    assert!(
        !acks::consume_action(&state.db, &other, &first.id, "fixture", &args)
            .await
            .unwrap()
    );
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        !acks::consume_action_in_session(
            &state.db,
            &other,
            &first.id,
            "fixture",
            &args,
            &mut session
        )
        .await
        .unwrap()
    );
    session.abort_transaction().await.unwrap();
    assert!(
        acks::consume_action(&state.db, &chat, &first.id, "fixture", &args)
            .await
            .unwrap()
    );
    invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    let browser_card = state
        .db
        .collection::<Document>(ACKS)
        .find_one(doc! {"_id":browser_card.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(browser_card.get_str("status").unwrap(), "pending");
}

#[tokio::test]
async fn invocation_watch_settlement_is_atomic_and_cancel_fences_late_producers() {
    use crate::models::nyxbot_channel::{NyxbotWatch, WATCHES_COLLECTION_NAME as WATCHES};
    use crate::services::{machine_service, nyxbot_watch_service as watches};
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_watch_settlement").await;
    let claim = admission.claim.unwrap();
    let watch = NyxbotWatch {
        id: Uuid::new_v4().to_string(),
        invocation_id: Some(admission.invocation.id.clone()),
        user_id: owner.clone(),
        conversation_id: claim.conversation.id.clone(),
        kind: "connect_link".into(),
        status: "pending".into(),
        trigger_prefill: None,
        trigger_id: None,
        platform: None,
        agent_id: None,
        channel_bot_id: None,
        connect_link_id: None,
        last_error: None,
        report_to: None,
        report_provenance: 1,
        checked_at: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    };
    state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .insert_one(&watch)
        .await
        .unwrap();
    assert!(
        machine_service::settle_watch(&state.db, &watch, "connected", "fixture".into(), None)
            .await
            .unwrap()
    );
    assert!(
        !machine_service::settle_watch(&state.db, &watch, "connected", "fixture".into(), None)
            .await
            .unwrap()
    );
    let row = engine::get(&state.db, &owner, &claim.conversation.id)
        .await
        .unwrap();
    assert_eq!(
        row.pending_events
            .iter()
            .filter(|e| e.invocation_id == watch.invocation_id)
            .count(),
        1
    );
    let stored = state
        .db
        .collection::<Document>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.get_str("status").unwrap(), "done");
    invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    let mut late = watch.clone();
    late.id = Uuid::new_v4().to_string();
    assert!(
        watches::upsert(
            &state.db,
            &owner,
            late.invocation_id.as_deref(),
            doc! {"_id":&late.id},
            doc! {"$setOnInsert":mongodb::bson::to_document(&late).unwrap()}
        )
        .await
        .is_err()
    );
    assert!(
        !machine_service::settle_watch(&state.db, &late, "connected", "late".into(), None)
            .await
            .unwrap()
    );
    assert!(
        engine::get(&state.db, &owner, &claim.conversation.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
}

#[tokio::test]
async fn invocation_completed_grants_allow_generation_switch_but_unused_actions_remain_recoverable()
{
    use crate::models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS, assistant_invocation::TURNS,
    };
    use mongodb::bson::{Document, doc};
    for kind in ["service", "account", "action"] {
        let (state, owner, admission) = admitted(&format!("invocation_allowed_{kind}")).await;
        let claim = admission.claim.unwrap();
        let card = Uuid::new_v4().to_string();
        state.db.collection::<Document>(ACKS).insert_one(doc! {
            "_id":&card,"user_id":&owner,"conversation_id":&claim.conversation.id,"invocation_id":&admission.invocation.id,
            "status":"allowed","kind":kind,"decided_by":"user","expires_at":mongodb::bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))
        }).await.unwrap();
        engine::force_stop(
            &state.db,
            &owner,
            &claim.conversation.id,
            &admission.invocation.turn_id,
        )
        .await
        .unwrap();
        state.db.collection::<Document>(TURNS).update_one(doc! {"_id":&admission.invocation.turn_id},doc! {"$set":{"state":"completed","hooks_pending":false},"$unset":{"error_code":""}}).await.unwrap();
        let snapshot =
            invocations::snapshot(&state.db, &owner, &admission.invocation.idempotency_key)
                .await
                .unwrap();
        if kind == "action" {
            assert_eq!(snapshot.status, "requires_review");
            assert_eq!(
                snapshot.error_code.as_deref(),
                Some("unused_owner_approval")
            );
            invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
                .await
                .unwrap();
            let cancelled =
                invocations::snapshot(&state.db, &owner, &admission.invocation.idempotency_key)
                    .await
                    .unwrap();
            assert_eq!(cancelled.status, "cancelled");
            assert_eq!(cancelled.error_code.as_deref(), Some("cancelled"));
        } else {
            assert_eq!(snapshot.status, "completed");
        }
        let (_, mut next, _) = parse(&body(&Uuid::new_v4().to_string(), "next"), &owner).unwrap();
        next.conversation_id = Some(claim.conversation.id);
        assert!(
            engine::begin_turn(&state.db, &owner, &next, &state.encryption_keys)
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
async fn invocation_expired_skill_producers_prevent_key_rotation_and_lost_notification_requires_review()
 {
    use crate::models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS, assistant_agent_learning as learning,
        assistant_invocation::TURNS,
    };
    use mongodb::bson::{Document, doc};
    let (state, owner, admission) = admitted("invocation_skill_generation").await;
    let claim = admission.claim.unwrap();
    engine::force_stop(
        &state.db,
        &owner,
        &claim.conversation.id,
        &admission.invocation.turn_id,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<Document>(TURNS)
        .update_one(
            doc! {"_id":&admission.invocation.turn_id},
            doc! {"$set":{"state":"completed","hooks_pending":false},"$unset":{"error_code":""}},
        )
        .await
        .unwrap();
    let proposal = Uuid::new_v4().to_string();
    state
        .db
        .collection::<Document>(learning::PROPOSALS_COLLECTION_NAME)
        .insert_one(doc! {"_id":&proposal,"owner_id":&owner,"status":"pending"})
        .await
        .unwrap();
    let card = Uuid::new_v4().to_string();
    state.db.collection::<Document>(ACKS).insert_one(doc! {
        "_id":&card,"user_id":&owner,"conversation_id":&claim.conversation.id,"invocation_id":&admission.invocation.id,
        "kind":"action","status":"expired","authored_skill":{"proposal_id":&proposal},
        "expires_at":mongodb::bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(1))
    }).await.unwrap();
    let (_, mut next, _) = parse(&body(&Uuid::new_v4().to_string(), "next"), &owner).unwrap();
    next.conversation_id = Some(claim.conversation.id.clone());
    assert!(matches!(
        engine::begin_turn(&state.db, &owner, &next, &state.encryption_keys).await,
        Err(AppError::Conflict(_))
    ));
    state
        .db
        .collection::<Document>(learning::PROPOSALS_COLLECTION_NAME)
        .update_one(doc! {"_id":&proposal}, doc! {"$set":{"status":"discarded"}})
        .await
        .unwrap();
    state.db.collection::<Document>(ACKS).update_one(doc! {"_id":&card},doc! {"$unset":{"authored_skill":""},"$set":{"kind":"skills","status":"allowed","skill_notification_pending":true}}).await.unwrap();
    assert!(matches!(
        engine::begin_turn(&state.db, &owner, &next, &state.encryption_keys).await,
        Err(AppError::Conflict(_))
    ));
    state.db.collection::<Document>(ACKS).update_one(doc! {"_id":&card},doc! {"$unset":{"skill_notification_pending":""},"$set":{"skill_notification_terminal":{"reason":"authority_lost"}}}).await.unwrap();
    let snapshot = invocations::snapshot(&state.db, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    assert_eq!(snapshot.status, "requires_review");
    assert_eq!(
        snapshot.error_code.as_deref(),
        Some("skill_notification_requires_review")
    );
    invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    assert!(
        engine::begin_turn(&state.db, &owner, &next, &state.encryption_keys)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn invocation_uncertain_skill_publication_does_not_claim_cancellation_complete() {
    use crate::models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS, assistant_agent_learning as learning,
        assistant_invocation::TURNS,
    };
    use mongodb::bson::{Document, doc};
    for status in ["publishing", "publication_failed"] {
        let (state, owner, admission) = admitted(&format!("invocation_publication_{status}")).await;
        let claim = admission.claim.unwrap();
        engine::force_stop(
            &state.db,
            &owner,
            &claim.conversation.id,
            &admission.invocation.turn_id,
        )
        .await
        .unwrap();
        state.db.collection::<Document>(TURNS).update_one(
            doc! {"_id": &admission.invocation.turn_id},
            doc! {"$set": {"state": "completed", "hooks_pending": false}, "$unset": {"error_code": ""}},
        ).await.unwrap();
        state.db.collection::<Document>(learning::PROPOSALS_COLLECTION_NAME).insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "owner_id": &owner, "status": status,
            "publication": {
                "invocation_id": &admission.invocation.id, "started": true,
                "uncertain_dispatch": true, "lease_expires_at": mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1)),
            },
        }).await.unwrap();
        // Cancellation expires this card. That cleanup reason must not mask
        // the unresolved external publication outcome.
        state.db.collection::<Document>(ACKS).insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": &owner,
            "conversation_id": &claim.conversation.id, "invocation_id": &admission.invocation.id,
            "status": "pending", "decider": "user", "kind": "action",
            "expires_at": mongodb::bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1)),
        }).await.unwrap();
        invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
            .await
            .unwrap();
        let snapshot =
            invocations::snapshot(&state.db, &owner, &admission.invocation.idempotency_key)
                .await
                .unwrap();
        assert_eq!(snapshot.status, "requires_review");
        assert!(!snapshot.terminal);
        assert_eq!(
            snapshot.error_code.as_deref(),
            Some("skill_publication_unconfirmed")
        );
    }
}

#[tokio::test]
async fn invocation_unresolved_orchestrator_decision_requires_review_and_remains_cancellable() {
    use crate::models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS, assistant_invocation::TURNS,
    };
    use mongodb::bson::{Document, doc};
    for expired in [false, true] {
        let (state, owner, admission) =
            admitted(&format!("invocation_orchestrator_decision_{expired}")).await;
        let claim = admission.claim.unwrap();
        engine::force_stop(
            &state.db,
            &owner,
            &claim.conversation.id,
            &admission.invocation.turn_id,
        )
        .await
        .unwrap();
        state.db.collection::<Document>(TURNS).update_one(
            doc! {"_id": &admission.invocation.turn_id},
            doc! {"$set": {"state": "completed", "hooks_pending": false}, "$unset": {"error_code": ""}},
        ).await.unwrap();
        state.db.collection::<Document>(ACKS).insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": &owner,
            "conversation_id": &claim.conversation.id, "invocation_id": &admission.invocation.id,
            "status": if expired {"expired"} else {"pending"}, "decider": "orchestrator", "kind": "service",
            "expires_at": mongodb::bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(if expired {-1} else {1})),
        }).await.unwrap();
        let snapshot =
            invocations::snapshot(&state.db, &owner, &admission.invocation.idempotency_key)
                .await
                .unwrap();
        assert_eq!(snapshot.status, "requires_review");
        assert!(!snapshot.terminal);
        assert!(snapshot.required_actions.is_empty());
        assert_eq!(
            snapshot.error_code.as_deref(),
            Some(if expired {
                "orchestrator_decision_expired"
            } else {
                "orchestrator_decision_unresolved"
            })
        );
        invocations::cancel(&state, &owner, &admission.invocation.idempotency_key)
            .await
            .unwrap();
        assert_eq!(
            invocations::snapshot(&state.db, &owner, &admission.invocation.idempotency_key)
                .await
                .unwrap()
                .status,
            "cancelled"
        );
    }
}

#[test]
fn invocation_input_cannot_supply_media_guest_or_channel_authority() {
    let key = Uuid::new_v4().to_string();
    let (_, start, _) = parse(&body(&key, "text only"), "owner").unwrap();
    assert!(start.media_event_ids.is_empty());
    assert!(start.attachment_ids.is_empty());
    assert!(!start.guest);
    assert!(start.channel.is_none());
    assert!(start.question_key.is_none());
    for (field, value) in [
        ("media_event_ids", json!(["source"])),
        ("attachment_ids", json!(["upload"])),
        ("guest", json!(true)),
        ("question_key", json!("caller-selected")),
        ("channel", json!({"platform":"lark"})),
    ] {
        let mut input = json!({"idempotency_key":key,"text":"text only"});
        input[field] = value;
        assert!(parse(&serde_json::to_vec(&input).unwrap(), "owner").is_err());
    }
}

#[tokio::test]
async fn invocation_admission_preserves_queued_channel_media_and_selection_metadata() {
    use crate::models::assistant_conversation::{COLLECTION_NAME as CONVERSATIONS, TurnOrigin};
    use crate::services::{assistant_team_service as team, async_service_operation as operations};
    use mongodb::bson::{self, Document, doc};
    let (state, owner, initial) = admitted("invocation_queued_media").await;
    let conversation = initial.claim.unwrap().conversation.id;
    invocations::cancel(&state, &owner, &initial.invocation.idempotency_key)
        .await
        .unwrap();
    let mut media = team::event("message", "queued owner photos".into(), None);
    media.source_event_ids = vec![Uuid::new_v4().to_string()];
    media.media_event_ids = media.source_event_ids.clone();
    media.media_declared = Some(2);
    media.media_sent_at = Some(Utc::now());
    let pending = bson::to_bson(&vec![media.clone()]).unwrap();
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &conversation},
            doc! {"$set":{"pending_events": &pending}},
        )
        .await
        .unwrap();
    let key = Uuid::new_v4().to_string();
    let input = serde_json::to_vec(&json!({
        "idempotency_key":key,"conversation_id":conversation,"text":"separate API work"
    }))
    .unwrap();
    let (_, start, digest) = parse(&input, &owner).unwrap();
    let admitted = Box::pin(invocations::admit(
        &state.db,
        &owner,
        &key,
        &digest,
        &start,
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    assert!(admitted.claim.is_some());
    let mut current = engine::get(&state.db, &owner, &conversation).await.unwrap();
    assert_eq!(bson::to_bson(&current.pending_events).unwrap(), pending);
    let turn = current.active_turn.as_ref().unwrap();
    assert_eq!(
        turn.invocation_id.as_deref(),
        Some(admitted.invocation.id.as_str())
    );
    assert!(turn.events.is_empty());
    let message = state
        .db
        .collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        )
        .find_one(doc! {"conversation_id":&conversation,"turn_id":&turn.turn_id,"role":"user"})
        .await
        .unwrap()
        .unwrap();
    assert!(message.media_event_ids.is_empty());
    assert!(message.media_pending.is_none());
    let tracked = team::event("notice", "invocation continuation".into(), None)
        .for_invocation(Some(&admitted.invocation.id));
    current.pending_events.insert(0, tracked.clone());
    let selected = operations::select_events(&mut current, TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, tracked.id);
    assert_eq!(bson::to_bson(&current.pending_events).unwrap(), pending);
    let selected = operations::select_events(&mut current, TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(
        bson::to_bson(&selected[0]).unwrap(),
        bson::to_bson(&media).unwrap()
    );
    assert!(selected[0].question_key.is_none());
    assert!(selected[0].invocation_id.is_none());
    assert!(current.pending_events.is_empty());
    state.db.drop().await.unwrap();
}
