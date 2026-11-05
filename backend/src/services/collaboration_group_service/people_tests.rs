use super::*;
use crate::handlers::collaboration_group as http;
use serde_json::json;

#[test]
fn collaboration_legacy_attribution_hides_departed_member_ids_only() {
    let former_person = uuid::Uuid::new_v4().to_string();
    let former_agent = uuid::Uuid::new_v4().to_string();
    let resource = uuid::Uuid::new_v4().to_string();
    let text = format!(
        "Group transcript:\nuser {former_person}: Hello\nagent {former_agent}: Reply\nDocument reference: {resource}\n"
    );
    let projected = people::public_text(&text, &[], &[]);
    assert!(!projected.contains(&former_person));
    assert!(!projected.contains(&former_agent));
    assert!(projected.contains("user \"Former participant\": Hello"));
    assert!(projected.contains("agent \"Former agent\": Reply"));
    assert!(projected.contains(&resource));
    assert!(projected.ends_with('\n'));
}

async fn names(f: &Fixture) {
    for (id, name) in [
        (&f.admin, "Kai"),
        (&f.member, "Shining"),
        (&f.viewer, "calvin tan"),
    ] {
        f.state
            .db
            .collection::<bson::Document>(USERS)
            .update_one(doc! {"_id":id}, doc! {"$set":{"display_name":name}})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn collaboration_people_mentions_resolve_exact_handles_and_refuse_ambiguous_names() {
    let (f, agent, group) = setup("collab_people_resolution").await;
    names(&f).await;
    let group = join(&f, &group, &f.member).await;
    let group = join(&f, &group, &f.viewer).await;
    let mut people = participants(&f.state.db, &group).await.unwrap();
    let shining = people.iter().find(|p| p.id == f.member).unwrap().clone();
    let handle = shining.handle.clone().unwrap();
    assert!(!handle.contains(&f.member));
    assert_eq!(
        people::resolve(
            &format!("Hi @{handle}"),
            &people,
            std::slice::from_ref(&agent),
            true
        )
        .unwrap()
        .ids,
        vec![f.member.clone()]
    );
    assert_eq!(
        people::resolve("@Shining @calvin-tan", &people, &[], true)
            .unwrap()
            .ids,
        vec![f.member.clone(), f.viewer.clone()]
    );
    people
        .iter_mut()
        .find(|p| p.id == f.viewer)
        .unwrap()
        .display_name = "Shining".into();
    assert!(people::resolve("@Shining", &people, &[], true).is_err());
    assert_eq!(
        people::resolve(&format!("@{handle}"), &people, &[], true)
            .unwrap()
            .ids,
        vec![f.member.clone()]
    );
    let mut namesake = agent.clone();
    namesake.name = "Shining".into();
    assert!(
        people::resolve(
            "@Shining",
            &[shining],
            std::slice::from_ref(&namesake),
            true
        )
        .is_err()
    );
    let ambiguous =
        people::resolve("@Shining", &people, std::slice::from_ref(&namesake), false).unwrap();
    assert!(ambiguous.ids.is_empty());
    assert!(svc::assistant_group_service::mentions(&ambiguous.agent_text, &[namesake]).is_empty());
    let removed = change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let people = participants(&f.state.db, &removed).await.unwrap();
    let mention = people::resolve(&format!("@{handle}"), &people, &[], true).unwrap();
    assert!(mention.ids.is_empty());
    assert!(mention.has_people);
    let unknown = people::resolve("@person-0000000000000000", &people, &[], true).unwrap();
    assert!(unknown.ids.is_empty());
    let unknown_uppercase =
        people::resolve("@PERSON-0000000000000000", &people, &[], true).unwrap();
    assert!(unknown_uppercase.ids.is_empty());
    assert!(unknown_uppercase.has_people);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_people_only_posts_have_no_execution_and_preserve_sticky_recipients() {
    let (f, agent, group) = setup("collab_people_no_execution").await;
    names(&f).await;
    let group = join(&f, &group, &f.member).await;
    f.state
        .db
        .collection::<AssistantGroup>(GROUPS)
        .update_one(
            doc! {"_id":&group.id},
            doc! {"$set":{format!("last_recipient_ids_by_actor.{}",f.admin):[&agent.id]}},
        )
        .await
        .unwrap();
    let people = participants(&f.state.db, &group).await.unwrap();
    let handle = people
        .iter()
        .find(|p| p.id == f.member)
        .unwrap()
        .handle
        .as_ref()
        .unwrap();
    for recipients in [None, Some(&[][..])] {
        let response = http::post(
            &f.state,
            &f.admin,
            &group.id,
            &format!("@{handle} hello"),
            recipients,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(response["request_id"], json!(null));
        assert_eq!(
            response["message"]["mentioned_participant_ids"],
            json!([f.member])
        );
        assert_eq!(response["addressed_agent_ids"], json!([]));
    }
    let stored = get(&f.state.db, &f.admin, &group.id).await.unwrap();
    assert_eq!(
        stored.last_recipient_ids_by_actor[&f.admin],
        vec![agent.id.clone()]
    );
    assert!(threads(&f.state.db, &group.id).await.unwrap().is_empty());
    for collection in [
        "assistant_group_requests",
        "assistant_acknowledgements",
        "assistant_agent_consents",
        "usage_meter",
        "billing_transactions",
    ] {
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    assert!(
        http::post(
            &f.state,
            &f.outsider,
            &group.id,
            &format!("@{handle} stranger"),
            None,
            &[]
        )
        .await
        .is_err()
    );
    let removed = change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let response = http::post(
        &f.state,
        &f.admin,
        &removed.id,
        &format!("@{handle} stale"),
        None,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(response["request_id"], json!(null));
    assert_eq!(response["message"]["mentioned_participant_ids"], json!([]));
    // No mention keeps the original sticky/lead contract.
    let request =
        requests::enqueue_ready(&f.state.db, &f.admin, &group.id, "Ordinary message", None)
            .await
            .unwrap();
    assert_eq!(request.pending_agent_ids, vec![agent.id]);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_mixed_mentions_keep_agent_routing_and_publish_people_ids() {
    let (f, agent, group) = setup("collab_people_mixed").await;
    names(&f).await;
    let group = join(&f, &group, &f.member).await;
    let request = requests::enqueue_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        &format!("@Shining @{} please help", agent.id),
        None,
    )
    .await
    .unwrap();
    assert_eq!(request.pending_agent_ids, vec![agent.id.clone()]);
    let messages = messages(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    assert_eq!(
        messages[0].mentioned_participant_ids,
        vec![f.member.clone()]
    );
    assert_eq!(messages[0].author_display_name.as_deref(), Some("Kai"));
    let started = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &group.id,
        &request.id,
        &agent.id,
    )
    .await
    .unwrap();
    let remaining = request.hops_remaining;
    let row = &started.claim.conversation;
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: "@Shining hello".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    http::settled(&f.state, row, "@Shining hello", None).await;
    let saved = f
        .state
        .db
        .collection::<crate::models::assistant_group::GroupRequest>(
            crate::models::assistant_group::REQUESTS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.hops_remaining, remaining);
    assert!(saved.pending_agent_ids.is_empty());
    let messages = super::messages(&f.state.db, &f.member, &group.id, None)
        .await
        .unwrap();
    assert_eq!(
        messages.last().unwrap().mentioned_participant_ids,
        vec![f.member]
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_people_context_is_public_requester_bound_in_shared_and_guest_lanes() {
    for guest in [false, true] {
        let (f, agent, group) = setup("collab_people_prompt").await;
        names(&f).await;
        let group = join(&f, &group, &f.member).await;
        let mut group = join(&f, &group, &f.viewer).await;
        if guest {
            group = change(
                &f,
                &f.admin,
                &group,
                Change::GuestAccess {
                    agent_id: agent.id.clone(),
                    enabled: true,
                    expected_revision: 1,
                    request_id: None,
                },
            )
            .await
            .group
            .unwrap();
        } else {
            share(&f, &agent, &[&f.member]).await;
        }
        f.state.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(doc! {"_id":&agent.id},doc! {"$set":{"memory":[{"id":"secret","text":"OWNER PRIVATE SENTINEL","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}]}}).await.unwrap();
        if !guest {
            svc::shared_agent_service::edit_private_note(
                &f.state.db,
                &f.member,
                &agent,
                Some("RECIPIENT PRIVATE SENTINEL"),
                None,
            )
            .await
            .unwrap();
        }
        // A public message from another participant is attributable by name.
        http::post(
            &f.state,
            &f.viewer,
            &group.id,
            "Hello from Calvin",
            Some(&[]),
            &[],
        )
        .await
        .unwrap();
        let request = requests::enqueue_ready(
            &f.state.db,
            &f.member,
            &group.id,
            &format!("@calvin-tan who am I, and who is Calvin? @{}", agent.id),
            None,
        )
        .await
        .unwrap();
        let started = runtime::begin_ready(
            &f.state.db,
            &f.state.encryption_keys,
            &f.member,
            &group.id,
            &request.id,
            &agent.id,
        )
        .await
        .unwrap();
        let row = &started.claim.conversation;
        assert_eq!(row.guest_turn, guest);
        let note = row.active_turn.as_ref().unwrap().note.as_ref().unwrap();
        assert!(
            note.contains("Original requester of this turn: \"Shining\" (@person-"),
            "{note}"
        );
        assert!(note.contains("\"calvin tan\" (@person-"));
        assert!(started.input.contains("user \"calvin tan\" (@person-"));
        assert!(started.input.contains("mentions \"calvin tan\" (@person-"));
        let agent_for_turn = runtime::agent_for_thread(&f.state.db, row).await.unwrap();
        let turn_notes =
            svc::assistant_nyxagent_runtime::turn_notes(&f.state, row, Some(&agent_for_turn), None)
                .await;
        assert!(turn_notes.contains("Original requester of this turn: \"Shining\""));
        for forbidden in [&f.admin, &f.member, &f.viewer, &f.outsider] {
            assert!(!note.contains(forbidden));
            assert!(!started.input.contains(forbidden));
        }
        assert!(!note.contains("@example.com"));
        assert!(!started.input.contains("@example.com"));
        assert!(agent_for_turn.memory.is_empty());
        for forbidden in ["OWNER PRIVATE SENTINEL", "RECIPIENT PRIVATE SENTINEL"] {
            assert!(!turn_notes.contains(forbidden));
            assert!(!started.input.contains(forbidden));
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn collaboration_people_mentions_live_events_reach_only_current_participants() {
    use svc::assistant_live::{AssistantLive, LiveEvent};
    let (f, _, group) = setup("collab_people_notify").await;
    let group = join(&f, &group, &f.member).await;
    let live = AssistantLive::default();
    let mut member = live.subscribe_owner(&f.member).unwrap();
    let mut outsider = live.subscribe_owner(&f.outsider).unwrap();
    let event = || LiveEvent::OrgGroup {
        id: group.id.clone(),
        user_id: String::new(),
    };
    let handle = participants(&f.state.db, &group)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.id == f.member)
        .unwrap()
        .handle
        .unwrap();
    http::post(
        &f.state,
        &f.admin,
        &group.id,
        &format!("@{handle}"),
        None,
        &[],
    )
    .await
    .unwrap();
    live.publish_resolved(&f.state.db, event()).await;
    assert!(
        matches!(member.events.try_recv().unwrap(),LiveEvent::OrgGroup{id,user_id} if id==group.id && user_id==f.member)
    );
    assert!(outsider.events.try_recv().is_err());
    change(
        &f,
        &f.admin,
        &group,
        Change::Remove {
            person_id: f.member.clone(),
        },
    )
    .await;
    http::post(
        &f.state,
        &f.admin,
        &group.id,
        &format!("@{handle}"),
        None,
        &[],
    )
    .await
    .unwrap();
    live.publish_resolved(&f.state.db, event()).await;
    assert!(member.events.try_recv().is_err());
    assert!(outsider.events.try_recv().is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_people_context_preserves_all_sixteen_participants() {
    let (f, agent, mut group) = setup("collab_people_full_roster").await;
    let template = f
        .state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":&f.member})
        .await
        .unwrap()
        .unwrap();
    for i in 1..16 {
        let mut person = template.clone();
        person.id = uuid::Uuid::new_v4().to_string();
        person.email = format!("{i}@example.invalid");
        person.display_name = Some(format!("Participant{i} {}", "x".repeat(65)));
        f.state
            .db
            .collection::<User>(USERS)
            .insert_one(&person)
            .await
            .unwrap();
        group = join(&f, &group, &person.id).await;
    }
    let started = runtime_start(&f, &f.admin, &group, &agent).await;
    let note = started.conversation.active_turn.unwrap().note.unwrap();
    assert!(note.contains("Participant15"));
    assert!(note.contains("For handoffs"));
    assert_eq!(note.matches("(@person-").count(), 17); // roster plus requester
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_agent_handles_and_legacy_uuid_handoffs_are_exact_without_id_prose() {
    let (f, agent, group) = setup("collaboration_public_handles").await;
    let target = specialist(&f, &f.admin, "second-agent").await;
    let group = change(
        &f,
        &f.admin,
        &group,
        Change::AddAgent {
            agent_id: target.id.clone(),
            proposed_by: f.admin.clone(),
        },
    )
    .await
    .group
    .unwrap();
    let participants = participants(&f.state.db, &group).await.unwrap();
    let roster = vec![agent.clone(), target.clone()];
    for token in [&target.id, &people::agent_handle(&target.id)] {
        let resolved =
            people::resolve(&format!("@{token} continue"), &participants, &roster, false).unwrap();
        assert_eq!(
            svc::assistant_group_service::mentions(&resolved.agent_text, &roster),
            vec![target.id.clone()]
        );
    }
    let unknown = uuid::Uuid::new_v4().to_string();
    let resolved = people::resolve(&format!("@{unknown}"), &participants, &roster, false).unwrap();
    assert!(svc::assistant_group_service::mentions(&resolved.agent_text, &roster).is_empty());
    let projected = people::public_text(
        &format!("@{} @{} @{unknown}", target.id, f.admin),
        &participants,
        &roster,
    );
    assert!(!projected.contains(&target.id));
    assert!(!projected.contains(&f.admin));
    assert!(!projected.contains(&unknown));
    assert!(projected.contains("@unknown-member"));
    let request = requests::enqueue_ready(
        &f.state.db,
        &f.admin,
        &group.id,
        &format!("@{} please help @{}", agent.id, f.admin),
        Some(std::slice::from_ref(&agent.id)),
    )
    .await
    .unwrap();
    let started = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.admin,
        &group.id,
        &request.id,
        &agent.id,
    )
    .await
    .unwrap();
    for id in [&agent.id, &target.id, &f.admin] {
        assert!(!started.input.contains(id));
        assert!(
            !started
                .claim
                .conversation
                .active_turn
                .as_ref()
                .unwrap()
                .note
                .as_ref()
                .unwrap()
                .contains(id)
        );
    }
    let history = runtime::history(&f.state.db, &started.claim.conversation, 20, None)
        .await
        .unwrap();
    assert!(
        history
            .iter()
            .all(|m| !m.text.contains(&agent.id) && !m.text.contains(&f.admin))
    );
    let text = format!("@{} is tracking this thread", target.id);
    svc::assistant_nyxagent::finish_turn(
        &f.state.db,
        &started.claim.conversation,
        &started.claim.conversation.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &svc::assistant_nyxagent::TurnResult {
            text: text.clone(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let resolved = people::resolve(&text, &participants, &roster, false).unwrap();
    for id in svc::assistant_group_service::mentions(&resolved.agent_text, &roster) {
        assert!(
            requests::handoff_ready(&f.state.db, &started.admission, &id)
                .await
                .unwrap()
        );
    }
    let request = f
        .state
        .db
        .collection::<crate::models::assistant_group::GroupRequest>(
            crate::models::assistant_group::REQUESTS_COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.pending_agent_ids, vec![target.id.clone()]);
    let public = http::messages(&f.state, &f.admin, &group.id, None)
        .await
        .unwrap();
    let reply = public["messages"].as_array().unwrap().last().unwrap()["text"]
        .as_str()
        .unwrap();
    assert!(reply.contains(&people::agent_handle(&target.id)));
    assert!(!reply.contains(&target.id));
    let unknown_post = http::post(
        &f.state,
        &f.admin,
        &group.id,
        &format!("@{unknown}"),
        None,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(unknown_post["request_id"], serde_json::Value::Null);
    assert_eq!(unknown_post["message"]["text"], "@unknown-member");
    f.state.db.drop().await.unwrap();
}
