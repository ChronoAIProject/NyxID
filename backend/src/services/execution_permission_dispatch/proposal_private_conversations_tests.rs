use super::*;
use proposals::v3::{AssistantVersion, CHOICE, READINESS};

async fn readiness(f: &mut Fixture, enabled: bool) {
    // Stop the fixture heartbeat so it cannot restore a capability during the test.
    if let Some(heartbeat) = f.heartbeat.take() {
        heartbeat.abort();
        let _ = heartbeat.await;
    }
    let mut update = doc! {"$set":{"observed_at":bson::DateTime::now()}};
    update.insert(
        if enabled { "$addToSet" } else { "$pull" },
        doc! {"capabilities":READINESS},
    );
    f.state
        .db
        .collection::<Document>("security_replicas")
        .update_many(doc! {}, update)
        .await
        .unwrap();
}

async fn new_thread(f: &mut Fixture, agent: Option<String>) -> AssistantConversation {
    let request = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: agent,
        conversation_id: None,
        text: "A new private conversation".into(),
        model: None,
        access_mode: None,
    };
    let mut start: engine::TurnStart = (&request).into();
    start.execution_enrollment = true;
    let row = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
        .await
        .unwrap();
    f.key = row.credential_api_key_id.clone();
    f.raw = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &row.id,
    )
    .await
    .unwrap()
    .unwrap()
    .raw_key
    .to_string();
    row
}

async fn saved(f: &Fixture, row: &ApprovalRequest) -> ApprovalGrant {
    let id = proposals::proposal(row).unwrap().grant_id.as_ref().unwrap();
    f.state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

fn old_choices_unchanged(row: &ApprovalRequest) {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct OldChoice {
        id: String,
        digest: String,
        validity: Validity,
        label: String,
    }
    let p = proposals::proposal(row).unwrap();
    for c in p.choices.iter().filter(|c| c.id != CHOICE) {
        assert!(c.conversation_scope.is_none());
        // Exact original digest preimage/domain, independent of the v3 writer.
        let expected = crate::services::audit_service::keyed_fingerprint(
            crate::services::audit_service::audit_chain_hmac_key(),
            b"execution-proposal-v1",
            &serde_json::to_vec(&json!({"scope":p.contract,"choice":c.validity})).unwrap(),
        )
        .unwrap();
        assert_eq!(c.digest, expected);
        let bytes = serde_json::to_vec(c).unwrap();
        let old: OldChoice = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            bytes,
            "old choice bytes are unchanged"
        );
    }
}

#[tokio::test]
async fn private_conversations_human_choice_reuses_only_new_private_occurrences() {
    for selection in [CHOICE, "persistent"] {
        let (mut f, first) =
            assistant_for_cohort(&format!("private_conversations_{selection}"), true).await;
        let (status, _) = f.call("/proxy/s/execution-fixture/status").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let request = proposal(&f).await;
        assert_eq!(
            AssistantVersion::of(&request).unwrap(),
            AssistantVersion::V3
        );
        old_choices_unchanged(&request);
        let input = decision(&request, selection);
        let approved = crate::handlers::approvals::decide_execution_request(
            axum::extract::State(f.state.clone()),
            test_utils::test_auth_user(&f.owner),
            axum::extract::Path(request.id.clone()),
            axum::Json(input),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(approved.status, "approved");
        let approved = proposal(&f).await;
        let grant = saved(&f, &approved).await;
        let contract = grant.execution.as_ref().unwrap().v1.as_ref().unwrap();
        assert_eq!(contract.subject.actor_user_id, f.owner);
        assert_eq!(
            contract.subject.principal,
            Principal::Agent(first.agent_id.clone().unwrap())
        );
        assert_eq!(contract.binding.connection_id, f.connection);
        assert_eq!(
            contract.context.audience,
            Audience::Private {
                user_id: f.owner.clone()
            }
        );
        assert_eq!(
            contract.context.conversation,
            if selection == CHOICE {
                ConversationScope::SameAudience
            } else {
                ConversationScope::Exact {
                    conversation_id: first.id.clone(),
                }
            }
        );
        let view = crate::handlers::approvals::get_grant_authorization(
            axum::extract::State(f.state.clone()),
            test_utils::test_auth_user(&f.owner),
            axum::extract::Path(grant.id.clone()),
        )
        .await
        .unwrap()
        .0
        .execution
        .unwrap();
        assert_eq!(
            view.context_scope,
            Some(if selection == CHOICE {
                "private_conversations"
            } else {
                "current_conversation"
            })
        );

        finish(&f, &first, "Waiting for review").await;
        let resumed = event_turn(&f, &first).await;
        let result = resume(&f, &resumed).await;
        assert!(result.contains("HTTP 200"), "{result}");
        assert_eq!(resume(&f, &resumed).await, result);
        assert_eq!(
            f.server.received_requests().await.unwrap().len(),
            1,
            "original effect never repeats"
        );
        finish(&f, &resumed, &result).await;
        assert!(
            proposals::proposal(&proposal(&f).await)
                .unwrap()
                .delivered_turn_id
                .is_some()
        );
        let second = new_thread(&mut f, first.agent_id.clone()).await;
        assert_ne!(first.id, second.id);
        assert_ne!(first.credential_api_key_id, second.credential_api_key_id);
        let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
        if selection == CHOICE {
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(count(&f, "approval_requests").await, 1);
            assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
        } else {
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            assert_eq!(count(&f, "approval_requests").await, 2);
            assert_eq!(
                f.server.received_requests().await.unwrap().len(),
                1,
                "old Persistent remains Exact"
            );
        }
        f.cleanup().await;
    }
}

#[tokio::test]
async fn private_conversations_old_readers_refuse_and_readiness_changes_deduplicate() {
    #[derive(serde::Deserialize)]
    struct OldEnvelope {
        version: u32,
        v1: Option<serde_json::Value>,
        v2: Option<serde_json::Value>,
    }
    let (mut f, first) = assistant("private_conversations_old_reader").await;
    readiness(&mut f, false).await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let old = proposal(&f).await;
    assert_eq!(AssistantVersion::of(&old).unwrap(), AssistantVersion::V1);
    old_choices_unchanged(&old);
    assert_eq!(proposals::proposal(&old).unwrap().choices.len(), 4);
    let old_bytes = bson::to_vec(old.execution_proposal.as_ref().unwrap()).unwrap();
    readiness(&mut f, true).await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(count(&f, "approval_requests").await, 1);
    assert_eq!(
        bson::to_vec(proposal(&f).await.execution_proposal.as_ref().unwrap()).unwrap(),
        old_bytes
    );
    finish(&f, &first, "Waiting").await;
    let second = new_thread(&mut f, first.agent_id.clone()).await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let new = f
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"execution_proposal.version":3})
        .await
        .unwrap()
        .unwrap();
    let original_reader: OldEnvelope =
        bson::from_document(bson::to_document(new.execution_proposal.as_ref().unwrap()).unwrap())
            .unwrap();
    assert_eq!(original_reader.version, 3);
    assert!(original_reader.v1.is_none() && original_reader.v2.is_none());
    // Actual pre-change decision lookup cannot select the new row.
    assert!(
        f.state
            .db
            .collection::<ApprovalRequest>("approval_requests")
            .find_one(doc! {
                "_id":&new.id,"user_id":&f.owner,"execution_proposal.version":{"$in":[1,2]}
            })
            .await
            .unwrap()
            .is_none()
    );
    assert!(proposals::legacy_refusal(&new).is_err());
    assert!(
        crate::services::approval_service::process_decision(
            &f.state.db,
            &f.state.config,
            &f.state.http_client,
            None,
            None,
            &new.id,
            true,
            Some(3600),
            Some("legacy-attempt"),
            "web",
        )
        .await
        .is_err()
    );
    assert_eq!(count(&f, "approval_grants").await, 0);
    assert_eq!(
        proposals::proposal(&new).unwrap().conversation_id,
        second.id
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn private_conversations_readiness_loss_blocks_decision_and_resume() {
    for phase in ["decision", "resume"] {
        let (mut f, first) = assistant(&format!("private_conversations_readiness_{phase}")).await;
        let _ = f.call("/proxy/s/execution-fixture/status").await;
        let request = proposal(&f).await;
        assert_eq!(
            AssistantVersion::of(&request).unwrap(),
            AssistantVersion::V3
        );
        if phase == "resume" {
            proposals::decide(&f.state, &f.owner, &request.id, &decision(&request, CHOICE))
                .await
                .unwrap();
            finish(&f, &first, "Waiting").await;
        }
        readiness(&mut f, false).await;
        if phase == "decision" {
            assert!(matches!(
                proposals::decide(&f.state, &f.owner, &request.id, &decision(&request, CHOICE))
                    .await,
                Err(AppError::SecurityReadinessUnavailable)
            ));
            assert_eq!(count(&f, "approval_grants").await, 0);
        } else {
            let resumed = event_turn(&f, &first).await;
            let egress = crate::handlers::assistant_nyxagent::http_egress(
                f.state.clone(),
                test_utils::test_auth_user(&f.owner),
            );
            assert!(matches!(
                proposals::resume_turn(&f.state, egress.as_ref(), &resumed).await,
                Err(AppError::SecurityReadinessUnavailable)
            ));
            assert_eq!(
                proposals::proposal(&proposal(&f).await).unwrap().state,
                "queued"
            );
        }
        assert_eq!(
            count(&f, "approval_requests").await,
            1,
            "no legacy downgrade"
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[tokio::test]
async fn private_conversations_scope_digest_rejects_tampering_without_a_grant() {
    let (f, _) = assistant("private_conversations_digest").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    let mut input = decision(&request, CHOICE);
    input.choice_digest = decision(&request, "persistent").choice_digest;
    assert!(
        proposals::decide(&f.state, &f.owner, &request.id, &input)
            .await
            .is_err()
    );
    input = decision(&request, CHOICE);
    input.expected_revision += 1;
    assert!(
        proposals::decide(&f.state, &f.owner, &request.id, &input)
            .await
            .is_err()
    );
    assert_eq!(count(&f, "approval_grants").await, 0);
    assert_eq!(proposal(&f).await.status, "pending");
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn private_conversations_inputs_and_other_agents_still_need_their_own_permission() {
    let (mut f, first) = assistant("private_conversations_scope").await;
    let _ = f.call("/proxy/s/execution-fixture/status").await;
    let request = proposal(&f).await;
    proposals::decide(&f.state, &f.owner, &request.id, &decision(&request, CHOICE))
        .await
        .unwrap();
    finish(&f, &first, "Waiting").await;
    let resumed = event_turn(&f, &first).await;
    let result = resume(&f, &resumed).await;
    finish(&f, &resumed, &result).await;
    new_thread(&mut f, first.agent_id.clone()).await;
    let (status, _) = f
        .call("/proxy/s/execution-fixture/status?different=resource")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    let (other, _) = crate::services::assistant_team_service::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        crate::services::assistant_team_service::CreateRequest {
            machines: None,
            logins: None,
            name: "different-agent".into(),
            description: "A separate specialist".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: crate::services::assistant_team_service::GrantTargets {
                service_ids: vec![f.connection.clone()],
                ..Default::default()
            },
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    new_thread(&mut f, Some(other.id)).await;
    let (status, _) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(count(&f, "approval_requests").await, 3);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn private_conversations_revocation_and_task_stop_fence_the_original_effect() {
    for change in ["revoke", "stop", "account"] {
        let (f, first) = assistant(&format!("private_conversations_fence_{change}")).await;
        let _ = f.call("/proxy/s/execution-fixture/status").await;
        let request = proposal(&f).await;
        let approved =
            proposals::decide(&f.state, &f.owner, &request.id, &decision(&request, CHOICE))
                .await
                .unwrap();
        finish(&f, &first, "Waiting").await;
        let resumed = event_turn(&f, &first).await;
        match change {
            "revoke" => {
                crate::services::approval_service::revoke_grant(
                    &f.state.db,
                    &f.owner,
                    &saved(&f, &approved).await.id,
                )
                .await
                .unwrap();
            }
            "stop" => {
                tasks::stop(&f.state.db, &resumed).await.unwrap();
            }
            _ => {
                f.state
                    .db
                    .collection::<Document>("user_services")
                    .update_one(
                        doc! {"_id":&f.connection},
                        doc! {"$set":{"api_key_id":Uuid::new_v4().to_string()}},
                    )
                    .await
                    .unwrap();
            }
        }
        let egress = crate::handlers::assistant_nyxagent::http_egress(
            f.state.clone(),
            test_utils::test_auth_user(&f.owner),
        );
        let outcome = proposals::resume_turn(&f.state, egress.as_ref(), &resumed).await;
        assert!(
            outcome.is_err()
                || outcome.unwrap().is_some_and(
                    |text| text.contains("not sent") || text.contains("Nothing was sent")
                ),
            "{change}"
        );
        assert!(
            f.server.received_requests().await.unwrap().is_empty(),
            "{change}"
        );
        assert_eq!(count(&f, "approval_requests").await, 1);
        f.cleanup().await;
    }
}
