//! Feedback route tests (Issue 9) and reply-context logging tests (addendum
//! A1). Shared fixtures initialize every assistant message/group field in one
//! place, so new model fields are added here once.
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use mongodb::bson::{self, Document, doc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::curation_tests::{Fixture, fixture, request};
use crate::{
    crypto::jwt,
    models::{
        assistant_group::{
            AssistantGroup, COLLECTION_NAME as GROUPS, GroupMessage,
            MESSAGES_COLLECTION_NAME as GROUP_MESSAGES,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as ASSISTANT_MESSAGES},
        assistant_message_feedback::{AssistantMessageFeedback, COLLECTION_NAME as FEEDBACK},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    test_utils,
};

mod fixtures {
    use crate::models::{
        assistant_conversation::TurnActivity,
        assistant_group::{AssistantGroup, GroupMessage},
        assistant_message::AssistantMessage,
    };
    use chrono::Utc;

    /// A completed assistant reply; override fields with struct update syntax.
    pub fn message(id: String, conversation_id: String, user_id: String) -> AssistantMessage {
        AssistantMessage {
            author: None,
            activity_total: 0,
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            error_detail: None,
            specialist_id: None,
            upstream_error_code: None,
            steering: None,
            voice: None,
            execution_pending: false,
            id,
            conversation_id,
            user_id,
            seq: 1,
            turn_id: "feedback-turn".into(),
            role: "assistant".into(),
            text: "reply".into(),
            status: "completed".into(),
            error_code: None,
            created_at: Utc::now(),
            activities: vec![],
            attachments: vec![],
            origin: None,
            via: None,
        }
    }

    pub fn activity(label: &str, status: &str) -> TurnActivity {
        TurnActivity {
            failure: None,
            machine: None,
            id: uuid::Uuid::new_v4().to_string(),
            label: label.into(),
            digest: None,
            status: status.into(),
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
        }
    }

    pub fn group_message(id: &str, group_id: &str, user_id: &str) -> GroupMessage {
        GroupMessage {
            recipient_agent_ids: Vec::new(),
            activities: vec![],
            org_group: false,
            author_user_id: None,
            author_display_name: None,
            request_id: None,
            attachments: vec![],
            id: id.into(),
            group_id: group_id.into(),
            user_id: user_id.into(),
            seq: 1,
            role: "agent".into(),
            agent_id: Some("feedback-agent".into()),
            agent_name: None,
            text: "reply".into(),
            created_at: Utc::now(),
        }
    }

    pub fn group(id: &str, owner: &str) -> AssistantGroup {
        AssistantGroup {
            last_recipient_ids: Vec::new(),
            last_recipient_ids_by_actor: Default::default(),
            id: id.into(),
            user_id: owner.into(),
            participant_user_ids: vec![owner.into()],
            created_by_user_id: None,
            name: "Feedback".into(),
            member_agent_ids: vec!["feedback-agent".into()],
            lead_agent_id: "feedback-agent".into(),
            created_by: "user".into(),
            message_count: 1,
            pending_agent_ids: vec![],
            hops_remaining: 0,
            followers: vec![],
            pending_checked_at: None,
            last_message_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[tokio::test]
async fn feedback_concurrent_first_puts_share_one_row() {
    // The unique index and barrier make concurrent first writes deterministic; repeat attempts are covered by the shared fixture index.
    let f = fixture("feedback-r3", false).await;
    let conversation_id = "nyxa-00000000000000000000000000000001";
    let message_id = "reply-feedback-r3";
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "turn-r3".into(),
            ..fixtures::message(message_id.into(), conversation_id.into(), f.owner.clone())
        },
    )
    .await
    .unwrap();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/{message_id}/feedback"
    );
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(8));
    let make = |vote: &'static str| {
        let state = f.state.clone();
        let token = f.human_token.clone();
        let path = path.clone();
        let barrier = barrier.clone();
        async move {
            barrier.wait().await;
            request(&state, "PUT", &path, &token, Some(json!({"vote": vote}))).await
        }
    };
    let mut tasks = Vec::new();
    for _ in 0..8 {
        tasks.push(tokio::spawn(make("up")));
    }
    for task in tasks {
        assert!(task.await.unwrap().0.is_success());
    }
    let count = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_message_feedback::COLLECTION_NAME)
        .count_documents(doc! {"user_id": &f.owner, "message_id": message_id})
        .await
        .unwrap();
    assert_eq!(count, 1);
    // One atomic upsert per (person, reply): exactly one write saw no
    // previous row, so exactly one `submitted` event is audited.
    let audits = feedback_audits(&f, message_id, 1).await;
    assert_eq!(audits.len(), 1, "{audits:?}");
    assert_eq!(audits[0]["event_data"]["change"], "submitted");
    let direct_message = "reply-feedback-r3-direct";
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            seq: 2,
            turn_id: "turn-direct".into(),
            ..fixtures::message(
                direct_message.into(),
                conversation_id.into(),
                f.owner.clone(),
            )
        },
    )
    .await
    .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_message_feedback::COLLECTION_NAME).insert_one(doc! {"_id":"existing-direct","user_id":&f.owner,"message_id":direct_message,"conversation_id":conversation_id,"vote":"up","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let direct_path = format!(
        "/api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/{direct_message}/feedback"
    );
    assert!(
        request(
            &f.state,
            "PUT",
            &direct_path,
            &f.human_token,
            Some(json!({"vote":"down"}))
        )
        .await
        .0
        .is_success()
    );
}

#[tokio::test]
async fn feedback_admin_counts_group_exact_per_day_through_service_and_route() {
    let f = fixture("feedback-r2", false).await;
    let day_a = chrono::DateTime::parse_from_rfc3339("2026-01-10T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let day_b = chrono::DateTime::parse_from_rfc3339("2026-01-11T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut rows = Vec::new();
    for (i, (vote, created)) in [("up", day_a), ("up", day_a), ("down", day_a), ("up", day_b)]
        .into_iter()
        .enumerate()
    {
        rows.push(AssistantMessageFeedback {
            id: format!("r2-{i}"),
            user_id: f.owner.clone(),
            conversation_id: None,
            group_id: None,
            message_id: format!("r2-message-{i}"),
            turn_id: None,
            vote: vote.into(),
            comment: None,
            context: None,
            created_at: created,
            updated_at: created,
        });
    }
    for i in 0..120 {
        rows.push(AssistantMessageFeedback {
            id: format!("r2-fill-{i}"),
            user_id: f.owner.clone(),
            conversation_id: None,
            group_id: None,
            message_id: format!("r2-fill-message-{i}"),
            turn_id: None,
            vote: "up".into(),
            comment: None,
            context: None,
            created_at: day_b,
            updated_at: day_b,
        });
    }
    f.state
        .db
        .collection::<AssistantMessageFeedback>(FEEDBACK)
        .insert_many(rows)
        .await
        .unwrap();
    let counts = crate::services::assistant_feedback_service::admin_counts(&f.state.db, doc! {})
        .await
        .unwrap();
    let a = counts.iter().find(|v| v.day == "2026-01-10").unwrap();
    let b = counts.iter().find(|v| v.day == "2026-01-11").unwrap();
    assert_eq!((a.up, a.down), (2, 1));
    assert_eq!((b.up, b.down), (121, 0));
    let (status, body) = request(
        &f.state,
        "GET",
        "/api/v1/admin/assistant/feedback",
        &f.human_token,
        None,
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let route_a = body["counts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["day"] == "2026-01-10")
        .unwrap();
    assert_eq!(
        (route_a["up"].as_i64(), route_a["down"].as_i64()),
        (Some(2), Some(1))
    );
}

#[tokio::test]
async fn feedback_admin_cursor_tie_break_and_invalid_cursor() {
    let f = fixture("feedback-b2-cursor", false).await;
    let created = Utc::now();
    let rows = (0..101)
        .map(|i| AssistantMessageFeedback {
            id: format!("b2-{i:03}"),
            user_id: f.owner.clone(),
            conversation_id: None,
            group_id: None,
            message_id: format!("b2-message-{i}"),
            turn_id: None,
            vote: "up".into(),
            comment: None,
            context: None,
            created_at: created,
            updated_at: created,
        })
        .collect::<Vec<_>>();
    f.state
        .db
        .collection::<AssistantMessageFeedback>(FEEDBACK)
        .insert_many(rows)
        .await
        .unwrap();
    let (status, body) = request(
        &f.state,
        "GET",
        "/api/v1/admin/assistant/feedback",
        &f.human_token,
        None,
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    assert_eq!(body["recent"].as_array().unwrap().len(), 100);
    let cursor = body["next_cursor"].as_str().unwrap();
    let (encoded_time, encoded_id) = cursor.split_once('|').unwrap();
    assert!(encoded_time.ends_with('Z'));
    assert!(!encoded_id.is_empty());
    let (status, next) = request(
        &f.state,
        "GET",
        &format!("/api/v1/admin/assistant/feedback?cursor={cursor}"),
        &f.human_token,
        None,
    )
    .await;
    assert!(status.is_success(), "{status} {next}");
    assert_eq!(next["recent"].as_array().unwrap().len(), 1);
    let (status, _) = request(
        &f.state,
        "GET",
        "/api/v1/admin/assistant/feedback?cursor=bad",
        &f.human_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn feedback_put_change_withdraw_and_ineligible_reply() {
    let f = fixture("feedback-b5-behavior", false).await;
    let conversation_id = "nyxa-00000000000000000000000000000002";
    let message_id = "b5-reply";
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "b5-turn".into(),
            ..fixtures::message(message_id.into(), conversation_id.into(), f.owner.clone())
        },
    )
    .await
    .unwrap();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/{message_id}/feedback"
    );
    assert!(
        request(
            &f.state,
            "PUT",
            &path,
            &f.human_token,
            Some(json!({"vote":"up","comment":"kept"}))
        )
        .await
        .0
        .is_success()
    );
    assert!(
        request(
            &f.state,
            "PUT",
            &path,
            &f.human_token,
            Some(json!({"vote":"down"}))
        )
        .await
        .0
        .is_success()
    );
    let row = f
        .state
        .db
        .collection::<AssistantMessageFeedback>(FEEDBACK)
        .find_one(doc! {"message_id": message_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.vote, "down");
    assert_eq!(row.comment.as_deref(), Some("kept"));
    let (status, body) = request(&f.state, "DELETE", &path, &f.human_token, None).await;
    assert!(status.is_success(), "{status} {body}");
    assert_eq!(
        f.state
            .db
            .collection::<AssistantMessageFeedback>(FEEDBACK)
            .count_documents(doc! {"message_id": message_id})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn feedback_vote_change_preserves_comment_without_comment_field() {
    let f = fixture("feedback-s2-comment", false).await;
    let message_id = "s2-reply";
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "s2-turn".into(),
            ..fixtures::message(
                message_id.into(),
                "nyxa-00000000000000000000000000000003".into(),
                f.owner.clone(),
            )
        },
    )
    .await
    .unwrap();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/nyxa-00000000000000000000000000000003/messages/{message_id}/feedback"
    );
    request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote":"up","comment":"keep"})),
    )
    .await;
    request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote":"down"})),
    )
    .await;
    let row = f
        .state
        .db
        .collection::<AssistantMessageFeedback>(FEEDBACK)
        .find_one(doc! {"message_id": message_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.comment.as_deref(), Some("keep"));
}

#[tokio::test]
async fn feedback_withdraw_is_idempotent_after_first_delete() {
    let f = fixture("feedback-s3-withdraw", false).await;
    let message_id = "s3-reply";
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "s3-turn".into(),
            ..fixtures::message(
                message_id.into(),
                "nyxa-00000000000000000000000000000004".into(),
                f.owner.clone(),
            )
        },
    )
    .await
    .unwrap();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/nyxa-00000000000000000000000000000004/messages/{message_id}/feedback"
    );
    request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote":"up"})),
    )
    .await;
    let (status, body) = request(&f.state, "DELETE", &path, &f.human_token, None).await;
    assert!(status.is_success(), "{status} {body}");
    assert!(
        request(&f.state, "DELETE", &path, &f.human_token, None)
            .await
            .0
            .is_success()
    );
    let mut audits = 0;
    for _ in 0..20 {
        audits = f
            .state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type": "assistant_feedback_withdrawn"})
            .await
            .unwrap();
        if audits == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(audits, 1);
}

#[tokio::test]
async fn feedback_put_rejects_unknown_conversation_as_not_found() {
    let f = fixture("feedback-s1-acl", false).await;
    let (status, _) = request(
        &f.state,
        "PUT",
        "/api/v1/assistant/nyxagent/conversations/not-owned/messages/missing/feedback",
        &f.human_token,
        Some(json!({"vote":"up"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn feedback_route_limits_one_user_burst() {
    let f = fixture("feedback-s4-limit", false).await;
    let mut limited = false;
    for i in 0..31 {
        let path =
            format!("/api/v1/assistant/nyxagent/conversations/missing-{i}/messages/m/feedback");
        if request(
            &f.state,
            "PUT",
            &path,
            &f.human_token,
            Some(json!({"vote":"up"})),
        )
        .await
        .0 == StatusCode::TOO_MANY_REQUESTS
        {
            limited = true;
            break;
        }
    }
    assert!(limited);
}

#[tokio::test]
async fn feedback_routes_reject_when_engine_flag_is_disabled() {
    let f = fixture("feedback-s5-engine-off", false).await;
    let message_id = "engine-gated-reply";
    let conversation_id = "nyxa-00000000000000000000000000000005";
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "turn".into(),
            ..fixtures::message(message_id.into(), conversation_id.into(), f.owner.clone())
        },
    )
    .await
    .unwrap();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/{message_id}/feedback"
    );
    let override_doc = crate::models::feature_flag_override::FeatureFlagOverride {
        definition_id: None,
        id: "s5-off".into(),
        org_user_id: None,
        flag_key: crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY.into(),
        target_kind: crate::models::feature_flag_override::FlagTargetKind::User,
        target_key: Some(f.owner.clone()),
        enabled: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        updated_by: f.owner.clone(),
    };
    f.state
        .db
        .collection::<crate::models::feature_flag_override::FeatureFlagOverride>(
            crate::models::feature_flag_override::COLLECTION_NAME,
        )
        .insert_one(override_doc)
        .await
        .unwrap();
    let (status, body) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote":"up"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"].as_str(), Some("not_found"));
    assert_eq!(
        body["message"].as_str(),
        Some("Not found: Assistant route not found.")
    );
    f.state
        .db
        .collection::<crate::models::feature_flag_override::FeatureFlagOverride>(
            crate::models::feature_flag_override::COLLECTION_NAME,
        )
        .delete_many(doc! {"_id":"s5-off"})
        .await
        .unwrap();
    let (status, _) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote":"up"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn feedback_rejects_user_failed_and_stopped_replies() {
    let f = fixture("feedback-ineligible", false).await;
    for (id, role, status) in [
        ("user", "user", "completed"),
        ("failed", "assistant", "failed"),
        ("stopped", "assistant", "failed"),
    ] {
        insert_feedback_message(
            &f,
            AssistantMessage {
                continuation_of: None,
                retry_of: None,
                retry_claimed_by: None,
                activity_count: None,
                turn_id: id.into(),
                role: role.into(),
                status: status.into(),
                error_code: (status == "failed").then_some(if id == "stopped" {
                    "cancelled".into()
                } else {
                    "failed".into()
                }),
                ..fixtures::message(
                    id.into(),
                    format!(
                        "nyxa-{:032x}",
                        if id == "user" {
                            101
                        } else if id == "failed" {
                            102
                        } else {
                            103
                        }
                    ),
                    f.owner.clone(),
                )
            },
        )
        .await
        .unwrap();
        let path = format!(
            "/api/v1/assistant/nyxagent/conversations/nyxa-{:032x}/messages/{id}/feedback",
            if id == "user" {
                101
            } else if id == "failed" {
                102
            } else {
                103
            }
        );
        assert_eq!(
            request(
                &f.state,
                "PUT",
                &path,
                &f.human_token,
                Some(json!({"vote":"up"}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn feedback_group_acl_rejects_non_participant_as_not_found() {
    let f = fixture("feedback-group-acl", false).await;
    let group_id = "nyxg-11111111111111111111111111111111";
    let message_id = "feedback-group-reply";
    let now = Utc::now();
    f.state
        .db
        .collection::<AssistantGroup>(GROUPS)
        .insert_one(AssistantGroup {
            last_recipient_ids: Vec::new(),
            last_recipient_ids_by_actor: std::collections::HashMap::new(),
            id: group_id.into(),
            user_id: f.owner.clone(),
            participant_user_ids: vec![f.owner.clone()],
            created_by_user_id: None,
            name: "Feedback".into(),
            member_agent_ids: vec!["agent".into()],
            lead_agent_id: "agent".into(),
            created_by: "user".into(),
            message_count: 1,
            pending_agent_ids: vec![],
            hops_remaining: 0,
            followers: vec![],
            pending_checked_at: None,
            last_message_at: Some(now),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<GroupMessage>(GROUP_MESSAGES)
        .insert_one(GroupMessage {
            recipient_agent_ids: Vec::new(),
            activities: vec![],
            org_group: false,
            author_user_id: None,
            author_display_name: None,
            request_id: None,
            attachments: vec![],
            id: message_id.into(),
            group_id: group_id.into(),
            user_id: f.owner.clone(),
            seq: 1,
            role: "agent".into(),
            agent_id: Some("agent".into()),
            agent_name: None,
            text: "reply".into(),
            created_at: now,
        })
        .await
        .unwrap();
    let path =
        format!("/api/v1/assistant/nyxagent/groups/{group_id}/messages/{message_id}/feedback");
    let (status, _) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote":"up"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        request(&f.state, "DELETE", &path, &f.human_token, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let other_id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_utils::test_user(&other_id, UserType::Person))
        .await
        .unwrap();
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&other_id).unwrap(),
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (status, body) = request(&f.state, "PUT", &path, &token, Some(json!({"vote":"up"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["message"].as_str(), Some("Not found: Group not found"));
    let (status, body) = request(&f.state, "DELETE", &path, &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["message"].as_str(), Some("Not found: Group not found"));
    let org_id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_utils::test_user(&org_id, UserType::Org))
        .await
        .unwrap();
    let org_group_id = "nyxg-22222222222222222222222222222222";
    let org_message_id = "feedback-org-reply";
    f.state
        .db
        .collection::<AssistantGroup>(GROUPS)
        .insert_one(AssistantGroup {
            last_recipient_ids: Vec::new(),
            last_recipient_ids_by_actor: std::collections::HashMap::new(),
            id: org_group_id.into(),
            user_id: org_id.clone(),
            participant_user_ids: vec![f.owner.clone()],
            created_by_user_id: Some(f.owner.clone()),
            name: "Org feedback".into(),
            member_agent_ids: vec!["agent".into()],
            lead_agent_id: "agent".into(),
            created_by: "user".into(),
            message_count: 1,
            pending_agent_ids: vec![],
            hops_remaining: 0,
            followers: vec![],
            pending_checked_at: None,
            last_message_at: Some(now),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<GroupMessage>(GROUP_MESSAGES)
        .insert_one(GroupMessage {
            recipient_agent_ids: Vec::new(),
            activities: vec![],
            org_group: true,
            author_user_id: None,
            author_display_name: None,
            request_id: None,
            attachments: vec![],
            id: org_message_id.into(),
            group_id: org_group_id.into(),
            user_id: org_id,
            seq: 1,
            role: "agent".into(),
            agent_id: Some("agent".into()),
            agent_name: None,
            text: "org reply".into(),
            created_at: now,
        })
        .await
        .unwrap();
    let org_path = format!(
        "/api/v1/assistant/nyxagent/groups/{org_group_id}/messages/{org_message_id}/feedback"
    );
    let (status, body) = request(
        &f.state,
        "PUT",
        &org_path,
        &token,
        Some(json!({"vote":"up"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["message"].as_str(), Some("Not found: Group not found"));
    let (status, body) = request(&f.state, "DELETE", &org_path, &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["message"].as_str(), Some("Not found: Group not found"));
}

#[tokio::test]
async fn feedback_admin_route_rejects_non_admin() {
    let f = fixture("feedback-admin-acl", false).await;
    f.state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! {"_id": &f.owner},
            doc! {"$set": {"is_admin": false, "role_ids": bson::Bson::Array(vec![])}},
        )
        .await
        .unwrap();
    assert_eq!(
        request(
            &f.state,
            "GET",
            "/api/v1/admin/assistant/feedback",
            &f.human_token,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn feedback_existing_conversation_rejects_other_user_put_and_delete() {
    let f = fixture("feedback-other-user", false).await;
    let conversation_id = "nyxa-00000000000000000000000000000006";
    let message_id = "owned-reply";
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "turn".into(),
            ..fixtures::message(message_id.into(), conversation_id.into(), f.owner.clone())
        },
    )
    .await
    .unwrap();
    let other_id = Uuid::new_v4().to_string();
    let other = test_utils::test_user(&other_id, UserType::Person);
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(other)
        .await
        .unwrap();
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&other_id).unwrap(),
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/{message_id}/feedback"
    );
    assert_eq!(
        request(&f.state, "PUT", &path, &token, Some(json!({"vote":"up"})))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&f.state, "DELETE", &path, &token, None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn feedback_conversation_delete_purges_existing_rows() {
    let f = fixture("feedback-conversation-purge", false).await;
    let id = "nyxa-00000000000000000000000000000007";
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .insert_one(doc! {
            "_id": id, "user_id": &f.owner, "title":"Purge", "model":"test",
            "credential_api_key_id":"unused", "message_count":0,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()
        })
        .await
        .unwrap();
    f.state.db.collection::<Document>(FEEDBACK).insert_one(doc! {
        "_id":"conversation-feedback", "user_id": &f.owner, "conversation_id":id,
        "message_id":"reply", "vote":"up", "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()
    }).await.unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>(FEEDBACK)
            .count_documents(doc! {"conversation_id":id})
            .await
            .unwrap(),
        1
    );
    crate::services::assistant_nyxagent::delete(&f.state.db, &f.owner, id)
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>(FEEDBACK)
            .count_documents(doc! {"conversation_id":id})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn feedback_rejects_message_from_another_owned_conversation() {
    let f = fixture("feedback-cross-conversation", false).await;
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "turn".into(),
            ..fixtures::message(
                "scoped-reply".into(),
                "nyxa-00000000000000000000000000000008".into(),
                f.owner.clone(),
            )
        },
    )
    .await
    .unwrap();
    let actual = "/api/v1/assistant/nyxagent/conversations/nyxa-00000000000000000000000000000008/messages/scoped-reply/feedback";
    assert_eq!(
        request(
            &f.state,
            "PUT",
            actual,
            &f.human_token,
            Some(json!({"vote":"up"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    // A second conversation the same owner owns: the message id is valid,
    // but it belongs to the first conversation, so it is not found here.
    let other = "nyxa-00000000000000000000000000000006";
    insert_feedback_message(
        &f,
        fixtures::message("other-reply".into(), other.into(), f.owner.clone()),
    )
    .await
    .unwrap();
    // Positive control: the second conversation is owned and votable.
    assert_eq!(
        request(
            &f.state,
            "PUT",
            &format!(
                "/api/v1/assistant/nyxagent/conversations/{other}/messages/other-reply/feedback"
            ),
            &f.human_token,
            Some(json!({"vote":"down"}))
        )
        .await
        .0,
        StatusCode::OK,
    );
    let wrong =
        format!("/api/v1/assistant/nyxagent/conversations/{other}/messages/scoped-reply/feedback");
    for method in ["PUT", "DELETE"] {
        let input = (method == "PUT").then(|| json!({"vote":"up"}));
        let (status, body) = request(&f.state, method, &wrong, &f.human_token, input).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {body}");
        assert_eq!(body["error"].as_str(), Some("not_found"));
        assert_eq!(
            body["message"].as_str(),
            Some("Not found: message not found")
        );
    }
    let stored = f
        .state
        .db
        .collection::<AssistantMessageFeedback>(FEEDBACK)
        .find_one(doc! {"message_id": "scoped-reply"})
        .await
        .unwrap()
        .expect("the vote made through the owning conversation survives");
    assert_eq!(
        stored.conversation_id.as_deref(),
        Some("nyxa-00000000000000000000000000000008")
    );
    assert_eq!(
        request(&f.state, "DELETE", actual, &f.human_token, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn feedback_revoked_org_member_cannot_vote_on_org_agent_thread() {
    let f = fixture("feedback-r4-org-revoke", false).await;
    let org = Uuid::new_v4().to_string();
    let member = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_utils::test_user(&org, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection::<User>(USERS)
        .insert_one(test_utils::test_user(&member, UserType::Person))
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .insert_one(
            bson::to_document(&test_utils::test_membership(
                &org,
                &member,
                crate::models::org_membership::OrgRole::Member,
                None,
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    let now = bson::DateTime::now();
    f.state.db.collection::<Document>("assistant_conversations").insert_one(doc! {"_id":"nyxa-0000000000000000000000000000000a","user_id":&member,"title":"Org","model":"test","credential_api_key_id":"unused","message_count":1,"agent_owner_id":&org,"created_at":now,"updated_at":now}).await.unwrap();
    insert_feedback_message(
        &f,
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            turn_id: "t".into(),
            ..fixtures::message(
                "org-reply".into(),
                "nyxa-0000000000000000000000000000000a".into(),
                member.clone(),
            )
        },
    )
    .await
    .unwrap();
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &Uuid::parse_str(&member).unwrap(),
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let path = "/api/v1/assistant/nyxagent/conversations/nyxa-0000000000000000000000000000000a/messages/org-reply/feedback";
    assert_eq!(
        request(&f.state, "PUT", path, &token, Some(json!({"vote":"up"})))
            .await
            .0,
        StatusCode::OK
    );
    f.state
        .db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .delete_many(doc! {"org_user_id":&org,"member_user_id":&member})
        .await
        .unwrap();
    let (status, body) = request(&f.state, "PUT", path, &token, Some(json!({"vote":"down"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["message"].as_str(),
        Some("Forbidden: Organization agent access was revoked or is no longer permitted")
    );
    let (status, body) = request(&f.state, "DELETE", path, &token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["message"].as_str(),
        Some("Forbidden: Organization agent access was revoked or is no longer permitted")
    );
}

async fn insert_feedback_message(
    f: &Fixture,
    message: AssistantMessage,
) -> mongodb::error::Result<mongodb::results::InsertOneResult> {
    f.state.db.collection::<Document>("assistant_conversations").update_one(
        doc! {"_id": &message.conversation_id},
        doc! {"$setOnInsert": {"user_id": &message.user_id, "title":"Feedback fixture", "model":"test", "credential_api_key_id":"unused", "message_count":1, "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now()}}
    ).upsert(true).await?;
    f.state
        .db
        .collection::<AssistantMessage>(ASSISTANT_MESSAGES)
        .insert_one(message)
        .await
}

#[cfg(test)]
mod a1_tests {
    use crate::models::assistant_message_feedback::AssistantMessageFeedback;
    #[test]
    fn legacy_feedback_row_without_context_deserializes() {
        let row: AssistantMessageFeedback = bson::from_document(bson::doc! {
            "_id":"legacy", "user_id":"u", "message_id":"m", "vote":"up",
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()
        })
        .unwrap();
        assert!(row.context.is_none());
    }

    #[test]
    fn partial_context_deserializes_with_defaults_and_bson_dates() {
        let at = chrono::Utc::now();
        let row: AssistantMessageFeedback = bson::from_document(bson::doc! {
            "_id":"partial", "user_id":"u", "message_id":"m", "vote":"down",
            "context": {"model":"m", "reply_created_at": bson::DateTime::from_chrono(at)},
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()
        })
        .unwrap();
        let context = row.context.unwrap();
        assert_eq!(context.model.as_deref(), Some("m"));
        assert_eq!(context.tool_calls, 0);
        assert!(context.tool_labels.is_empty());
        assert_eq!(
            context.reply_created_at.unwrap().timestamp_millis(),
            at.timestamp_millis()
        );
        // Stored as a BSON date, not a string.
        let encoded = bson::to_document(&context).unwrap();
        assert!(matches!(
            encoded.get("reply_created_at"),
            Some(bson::Bson::DateTime(_))
        ));
    }
}

/// Captures `nyxid::assistant_feedback` lines written on this test's thread.
#[derive(Clone, Default)]
struct LogCapture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for LogCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl LogCapture {
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        let writer = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::set_default(subscriber)
    }
    /// Feedback lines written since the last call.
    fn take(&self) -> Vec<String> {
        let bytes = std::mem::take(&mut *self.0.lock().unwrap());
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .filter(|line| line.contains(crate::services::assistant_feedback_service::LOG_TARGET))
            .map(str::to_owned)
            .collect()
    }
}

/// Waits for the fire-and-forget audit writes, then returns the rows for one
/// message in insertion order.
async fn feedback_audits(f: &Fixture, message_id: &str, expected: usize) -> Vec<Value> {
    let mut rows = Vec::new();
    for _ in 0..40 {
        let docs: Vec<Document> = futures::TryStreamExt::try_collect(
            f.state
                .db
                .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .find(doc! {
                    "event_type": {"$in": ["assistant_feedback_submitted", "assistant_feedback_withdrawn"]},
                    "event_data.message_id": message_id,
                })
                .sort(doc! {"created_at": 1, "_id": 1})
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        rows = docs
            .into_iter()
            .map(|row| serde_json::to_value(row).unwrap())
            .collect();
        if rows.len() >= expected {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    // Give a stray extra write a chance to land before callers assert counts.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    rows
}

async fn insert_thread(f: &Fixture, conversation_id: &str, agent_id: &str, model: &str) {
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .insert_one(doc! {
            "_id": conversation_id, "user_id": &f.owner, "title": "Context",
            "model": model, "credential_api_key_id": "unused", "message_count": 2,
            "agent_id": agent_id, "role": "orchestrator",
            "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn feedback_vote_persists_reply_context_and_logs_metadata_only() {
    let f = fixture("feedback-a1-context", false).await;
    let conversation_id = "nyxa-000000000000000000000000000000c1";
    // A NyxBot thread carries its NyxBot agent id: kind comes from the
    // agent, and an owner's per-agent override is the effective mode.
    let nyxbot = crate::services::assistant_team_service::ensure_nyxbot(&f.state.db, &f.owner)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &nyxbot.id},
            doc! {"$set": {"access_mode_override": "autopilot"}},
        )
        .await
        .unwrap();
    insert_thread(&f, conversation_id, &nyxbot.id, "model-ctx").await;
    let asked = Utc::now() - Duration::seconds(10);
    let answered = asked + Duration::milliseconds(1500);
    for message in [
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            role: "user".into(),
            turn_id: "turn-ctx".into(),
            text: "PROMPT-SENTINEL".into(),
            created_at: asked,
            ..fixtures::message("ctx-user".into(), conversation_id.into(), f.owner.clone())
        },
        AssistantMessage {
            continuation_of: None,
            retry_of: None,
            retry_claimed_by: None,
            activity_count: None,
            seq: 2,
            turn_id: "turn-ctx".into(),
            text: "REPLY-SENTINEL".into(),
            created_at: answered,
            activities: vec![
                fixtures::activity("github__list_issues", "completed"),
                fixtures::activity("github__list_issues", "completed"),
                fixtures::activity("mail__send", "error"),
            ],
            ..fixtures::message("ctx-reply".into(), conversation_id.into(), f.owner.clone())
        },
    ] {
        f.state
            .db
            .collection::<AssistantMessage>(ASSISTANT_MESSAGES)
            .insert_one(message)
            .await
            .unwrap();
    }
    let logs = LogCapture::default();
    let _guard = logs.install();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/ctx-reply/feedback"
    );
    let (status, body) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote": "down", "comment": "COMMENT-SENTINEL"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = f
        .state
        .db
        .collection::<AssistantMessageFeedback>(FEEDBACK)
        .find_one(doc! {"message_id": "ctx-reply"})
        .await
        .unwrap()
        .unwrap();
    let context = row.context.expect("reply context is persisted");
    assert_eq!(context.surface.as_deref(), Some("conversation"));
    assert_eq!(context.agent_id.as_deref(), Some(nyxbot.id.as_str()));
    assert_eq!(context.agent_kind.as_deref(), Some("nyxbot"));
    assert_eq!(context.model.as_deref(), Some("model-ctx"));
    assert_eq!(context.access_mode.as_deref(), Some("autopilot"));
    assert_eq!(context.reply_status.as_deref(), Some("completed"));
    assert_eq!(context.turn_id.as_deref(), Some("turn-ctx"));
    assert_eq!(context.latency_ms, Some(1500));
    assert_eq!(context.tool_calls, 3);
    assert_eq!(context.tool_errors, 1);
    assert_eq!(context.tool_labels, ["github__list_issues", "mail__send"]);
    assert_eq!(context.reply_chars, "REPLY-SENTINEL".len() as u32);
    assert_eq!(
        context.reply_created_at.map(|at| at.timestamp_millis()),
        Some(answered.timestamp_millis())
    );
    let lines = logs.take();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    for expected in [
        "event=\"submitted\"",
        "ctx-reply",
        &row.id,
        conversation_id,
        "vote=down",
        "model=\"model-ctx\"",
        "agent_kind=\"nyxbot\"",
        "latency_ms=1500",
        "tool_errors=1",
        "tool_labels=github__list_issues,mail__send",
        "has_comment=true",
        "comment_chars=16",
    ] {
        assert!(
            line.contains(expected),
            "feedback log line is missing {expected}"
        );
    }
    for sentinel in ["COMMENT-SENTINEL", "REPLY-SENTINEL", "PROMPT-SENTINEL"] {
        assert!(
            !line.contains(sentinel),
            "{sentinel} leaked into the feedback log line"
        );
    }
    let audits = feedback_audits(&f, "ctx-reply", 1).await;
    assert_eq!(audits.len(), 1);
    let details = &audits[0]["event_data"];
    assert_eq!(details["change"], "submitted");
    assert_eq!(details["previous_vote"], Value::Null);
    assert_eq!(details["agent_kind"], "nyxbot");
    assert_eq!(details["agent_id"], nyxbot.id.as_str());
    assert_eq!(details["model"], "model-ctx");
    assert_eq!(details["tool_errors"], 1);
    assert_eq!(details["has_comment"], true);
    assert!(!audits[0].to_string().contains("COMMENT-SENTINEL"));
}

#[tokio::test]
async fn feedback_changes_are_classified_and_idempotent_repeats_are_silent() {
    let f = fixture("feedback-a1-classify", false).await;
    let conversation_id = "nyxa-000000000000000000000000000000c2";
    insert_feedback_message(
        &f,
        fixtures::message("cls-reply".into(), conversation_id.into(), f.owner.clone()),
    )
    .await
    .unwrap();
    let logs = LogCapture::default();
    let _guard = logs.install();
    let path = format!(
        "/api/v1/assistant/nyxagent/conversations/{conversation_id}/messages/cls-reply/feedback"
    );
    let put = |body: Value| {
        let state = f.state.clone();
        let token = f.human_token.clone();
        let path = path.clone();
        async move {
            let (status, body) = request(&state, "PUT", &path, &token, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
    };
    put(json!({"vote": "up"})).await;
    let first = logs.take();
    assert_eq!(first.len(), 1);
    assert!(first[0].contains("event=\"submitted\""), "{first:?}");
    put(json!({"vote": "down"})).await;
    let changed = logs.take();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].contains("event=\"changed\""), "{changed:?}");
    assert!(changed[0].contains("previous_vote=\"up\""), "{changed:?}");
    put(json!({"vote": "down", "comment": "too slow"})).await;
    let commented = logs.take();
    assert_eq!(commented.len(), 1);
    assert!(commented[0].contains("event=\"comment_updated\""));
    // Same vote, no comment field: idempotent, so no log line and no audit.
    put(json!({"vote": "down"})).await;
    // Same vote, same comment: also idempotent.
    put(json!({"vote": "down", "comment": "  too slow "})).await;
    assert!(logs.take().is_empty(), "idempotent repeats log nothing");
    let audits = feedback_audits(&f, "cls-reply", 3).await;
    let mut changes: Vec<_> = audits
        .iter()
        .map(|a| {
            (
                a["event_data"]["change"].as_str().unwrap().to_owned(),
                a["event_data"]["previous_vote"].clone(),
            )
        })
        .collect();
    // Audit writes are fire-and-forget; compare without relying on order.
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        changes,
        [
            ("changed".to_owned(), json!("up")),
            ("comment_updated".to_owned(), json!("down")),
            ("submitted".to_owned(), Value::Null),
        ]
    );
    // An explicit empty comment clears the stored one; whitespace counts as empty.
    put(json!({"vote": "down", "comment": "   "})).await;
    let cleared = logs.take();
    assert_eq!(cleared.len(), 1);
    assert!(cleared[0].contains("has_comment=false"), "{cleared:?}");
    // Withdraw logs the stored context and audits the previous vote.
    let (status, _) = request(&f.state, "DELETE", &path, &f.human_token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let withdrawn = logs.take();
    assert_eq!(withdrawn.len(), 1);
    for expected in [
        "event=\"withdrawn\"",
        "vote=down",
        "previous_vote=\"down\"",
        "has_comment=false",
        "has_context=true",
        "surface=\"conversation\"",
        "model=\"test\"",
        conversation_id,
    ] {
        assert!(
            withdrawn[0].contains(expected),
            "missing {expected}: {withdrawn:?}"
        );
    }
    let audits = feedback_audits(&f, "cls-reply", 5).await;
    assert_eq!(
        audits.len(),
        5,
        "the cleared comment and the withdrawal add one row each"
    );
    let last = audits
        .iter()
        .find(|a| a["event_type"] == "assistant_feedback_withdrawn")
        .expect("withdrawal audit");
    assert_eq!(last["event_data"]["previous_vote"], "down");
    assert_eq!(last["event_data"]["change"], "withdrawn");
    assert_eq!(last["event_data"]["model"], "test");
}

#[tokio::test]
async fn feedback_group_vote_persists_group_context() {
    let f = fixture("feedback-a1-group", false).await;
    let group_id = "nyxg-000000000000000000000000000000c3";
    f.state
        .db
        .collection::<AssistantGroup>(GROUPS)
        .insert_one(fixtures::group(group_id, &f.owner))
        .await
        .unwrap();
    f.state
        .db
        .collection::<GroupMessage>(GROUP_MESSAGES)
        .insert_one(GroupMessage {
            seq: 4,
            text: "group reply".into(),
            activities: vec![
                fixtures::activity("calendar__list", "completed"),
                fixtures::activity("calendar__list", "error"),
            ],
            ..fixtures::group_message("group-reply", group_id, &f.owner)
        })
        .await
        .unwrap();
    let path =
        format!("/api/v1/assistant/nyxagent/groups/{group_id}/messages/group-reply/feedback");
    let (status, body) = request(
        &f.state,
        "PUT",
        &path,
        &f.human_token,
        Some(json!({"vote": "down", "comment": "wrong calendar"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = f
        .state
        .db
        .collection::<AssistantMessageFeedback>(FEEDBACK)
        .find_one(doc! {"message_id": "group-reply"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.group_id.as_deref(), Some(group_id));
    assert_eq!(row.comment.as_deref(), Some("wrong calendar"));
    let context = row.context.expect("group context is persisted");
    assert_eq!(context.surface.as_deref(), Some("group"));
    assert_eq!(context.agent_id.as_deref(), Some("feedback-agent"));
    assert_eq!(context.reply_seq, Some(4));
    assert_eq!(context.tool_calls, 2);
    assert_eq!(context.tool_errors, 1);
    assert_eq!(context.tool_labels, ["calendar__list"]);
    assert_eq!(context.reply_chars, "group reply".len() as u32);
    assert!(!context.org_group);
    // Conversation-only fields stay absent.
    assert!(context.model.is_none() && context.agent_kind.is_none());
    assert!(context.latency_ms.is_none() && context.turn_id.is_none());
}

#[tokio::test]
async fn feedback_admin_lists_legacy_rows_and_filters_context_metadata() {
    let f = fixture("feedback-a1-admin-filter", false).await;
    let now = Utc::now();
    let rows = [
        (
            "a1-match",
            doc! {"model":"model-a","agent_kind":"specialist","error_code":"failed","tool_labels":["x__y"],"latency_ms": 42_i64},
        ),
        (
            "a1-other-model",
            doc! {"model":"model-b","agent_kind":"specialist","error_code":"failed"},
        ),
        (
            "a1-no-error",
            doc! {"model":"model-a","agent_kind":"specialist"},
        ),
        (
            "a1-nyxbot",
            doc! {"model":"model-a","agent_kind":"nyxbot","error_code":"failed"},
        ),
    ];
    for (offset, (id, context)) in rows.into_iter().enumerate() {
        let at = bson::DateTime::from_chrono(now - Duration::seconds(offset as i64));
        f.state
            .db
            .collection::<Document>(FEEDBACK)
            .insert_one(doc! {
                "_id": id, "user_id": &f.owner, "message_id": id, "vote": "down",
                "context": context, "created_at": at, "updated_at": at,
            })
            .await
            .unwrap();
    }
    let legacy_at = bson::DateTime::from_chrono(now - Duration::seconds(10));
    f.state
        .db
        .collection::<Document>(FEEDBACK)
        .insert_one(doc! {
            "_id": "a1-legacy", "user_id": &f.owner, "message_id": "a1-legacy", "vote": "up",
            "comment": "old", "created_at": legacy_at, "updated_at": legacy_at,
        })
        .await
        .unwrap();
    let ids = |body: &Value| -> Vec<String> {
        body["recent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["message_id"].as_str().unwrap().to_owned())
            .collect()
    };
    let (status, all) = request(
        &f.state,
        "GET",
        "/api/v1/admin/assistant/feedback",
        &f.human_token,
        None,
    )
    .await;
    assert!(status.is_success(), "{status} {all}");
    assert_eq!(
        ids(&all),
        [
            "a1-match",
            "a1-other-model",
            "a1-no-error",
            "a1-nyxbot",
            "a1-legacy"
        ]
    );
    let legacy = &all["recent"][4];
    assert_eq!(legacy["context"], Value::Null);
    assert_eq!(legacy["comment"], "old");
    let matched = &all["recent"][0]["context"];
    assert_eq!(matched["model"], "model-a");
    assert_eq!(matched["tool_labels"], json!(["x__y"]));
    assert_eq!(matched["latency_ms"], 42);
    assert_eq!(matched["tool_calls"], 0, "absent counters default");
    for (query, expected) in [
        (
            "model=model-a",
            vec!["a1-match", "a1-no-error", "a1-nyxbot"],
        ),
        ("agent_kind=nyxbot", vec!["a1-nyxbot"]),
        (
            "has_error=true",
            vec!["a1-match", "a1-other-model", "a1-nyxbot"],
        ),
        (
            "model=model-a&agent_kind=specialist&has_error=true",
            vec!["a1-match"],
        ),
    ] {
        let (status, body) = request(
            &f.state,
            "GET",
            &format!("/api/v1/admin/assistant/feedback?{query}"),
            &f.human_token,
            None,
        )
        .await;
        assert!(status.is_success(), "{query}: {status} {body}");
        assert_eq!(ids(&body), expected, "{query}");
    }
}

#[tokio::test]
async fn history_hydration_survives_a_malformed_context() {
    let f = fixture("feedback-a1-history", false).await;
    let ids = ["hydrate-a".to_owned(), "hydrate-b".to_owned()];
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<Document>(FEEDBACK)
        .insert_one(doc! {
            "_id": "malformed", "user_id": &f.owner, "message_id": "hydrate-a", "vote": "up",
            "context": {"tool_calls": "not-a-number"}, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    let states = crate::services::assistant_feedback_service::states_for_messages(
        &f.state.db,
        &f.owner,
        &ids,
    )
    .await
    .unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].message_id, "hydrate-a");
    assert_eq!(states[0].vote, "up");
    // The full model would refuse this row; hydration must not.
    assert!(
        f.state
            .db
            .collection::<AssistantMessageFeedback>(FEEDBACK)
            .find_one(doc! {"_id": "malformed"})
            .await
            .is_err()
    );
}
