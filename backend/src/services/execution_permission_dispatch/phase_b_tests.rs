use super::*;
use crate::models::{
    agent_operation_scope::{AgentOperationScope, ScopedOperation},
    assistant_conversation::AssistantConversation,
    downstream_service::ProxyOperationRule,
};

pub(crate) async fn enroll(db: &Database, actor: &str) {
    super::super::security_readiness_service::ready_test_replica(db).await;
    super::super::feature_flag_admin_service::mutate(
        db,
        super::super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
        super::super::feature_flag_admin_service::Mutation::Override {
            kind: crate::models::feature_flag_override::FlagTargetKind::User,
            target_id: Some(actor.into()),
            enabled: Some(true),
        },
        &super::super::audit_service::AuditActor {
            user_id: actor.into(),
            api_key_id: None,
            api_key_name: None,
            ip_address: None,
            user_agent: None,
        },
    )
    .await
    .unwrap();
}

/// Exercise the same Phase B contracts without a materialized person marker.
pub(crate) async fn enroll_global(db: &Database, actor: &str) {
    super::super::security_readiness_service::ready_test_replica(db).await;
    super::super::feature_flag_admin_service::mutate(
        db,
        super::super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
        super::super::feature_flag_admin_service::Mutation::Override {
            kind: crate::models::feature_flag_override::FlagTargetKind::Global,
            target_id: None,
            enabled: Some(true),
        },
        &super::super::audit_service::AuditActor {
            user_id: actor.into(),
            api_key_id: None,
            api_key_name: None,
            ip_address: None,
            user_agent: None,
        },
    )
    .await
    .unwrap();
    super::super::execution_permission_enrollment::snapshot::Cache::for_database(db)
        .refresh(db)
        .await
        .unwrap();
    assert!(
        db.collection::<Document>("users")
            .find_one(doc! {"_id":actor,"execution_permission_enrollment":null})
            .await
            .unwrap()
            .is_some()
    );
}

pub(crate) async fn enroll_cohort(db: &Database, actor: &str, global: bool) {
    if global {
        enroll_global(db, actor).await;
    } else {
        enroll(db, actor).await;
    }
}

/// Keep legacy gap tests bounded while preserving the actual approval path.
pub(crate) async fn legacy_review(state: &crate::AppState, actor: &str) {
    super::super::assistant_settings_service::update(
        &state.db,
        actor,
        super::super::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let channel = super::super::notification_service::get_or_create_channel(&state.db, actor)
        .await
        .unwrap();
    state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"_id":channel.id},
            doc! {"$set":{"approval_timeout_secs":1}},
        )
        .await
        .unwrap();
}

pub(crate) async fn task(db: &Database, row: &AssistantConversation) -> TaskAdmission {
    let root = db
        .collection::<Document>("assistant_messages")
        .find_one(doc! {"conversation_id":&row.id,"role":"user"})
        .await
        .unwrap()
        .unwrap();
    let task = TaskAdmission {
        channel_source: None,
        version: VERSION,
        task: TaskRef {
            root_kind: TaskRootKind::Message,
            root_id: root.get_str("_id").unwrap().into(),
        },
        actor_user_id: row.user_id.clone(),
        audience: Audience::Private {
            user_id: row.user_id.clone(),
        },
        epoch: 1,
        hard_deadline: Utc::now() + chrono::Duration::hours(24),
        cancelled: false,
        parent_turn_id: None,
    };
    let value = bson::to_bson(&task).unwrap();
    db.collection::<Document>("assistant_messages")
        .update_one(
            doc! {"_id":&task.task.root_id},
            doc! {"$set":{"execution_task":&value}},
        )
        .await
        .unwrap();
    db.collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn.execution_task":value}},
        )
        .await
        .unwrap();
    task
}

pub(crate) async fn grant(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    connection: &str,
    method: &str,
    path: &str,
) -> ApprovalGrant {
    let context = auth.execution_permission.as_ref().unwrap();
    let key = auth.api_key_id.as_deref().unwrap();
    let service =
        super::super::user_service_service::find_user_service_by_id(&state.db, connection)
            .await
            .unwrap()
            .unwrap();
    let binding = binding(
        &state.db,
        &state.encryption_keys,
        context,
        auth.assistant_chat.as_deref(),
        key,
        connection,
        &service.user_id,
    )
    .await
    .unwrap();
    let mut g = contract::tests::fixture();
    g.id = Uuid::new_v4().to_string();
    g.user_id = context.actor.clone();
    g.requester_id = key.into();
    g.service_id = service
        .catalog_service_id
        .clone()
        .unwrap_or_else(|| connection.into());
    let p = g.execution.as_mut().unwrap().v1.as_mut().unwrap();
    p.subject = Subject {
        actor_user_id: context.actor.clone(),
        principal: context.principal.clone(),
        lane: ExecutionLane::Owner,
    };
    p.binding = binding;
    p.context = ExecutionContext {
        audience: Audience::Private {
            user_id: context.actor.clone(),
        },
        conversation: ConversationScope::SameAudience,
    };
    p.operations = AgentOperationScope {
        revision: 1,
        // Match the coordinator/proposal's concrete service identity, including
        // the connection ID used by a custom service's synthetic catalog view.
        catalog_service_id: Some(service.catalog_service_id.unwrap_or(service.id)),
        operations: vec![ScopedOperation {
            endpoint_id: None,
            rule: ProxyOperationRule {
                method: method.into(),
                path_template: path.into(),
                ..Default::default()
            },
            ..Default::default()
        }],
    };
    p.satisfies = vec![
        ExecutionFloor::AccessModeRisk,
        ExecutionFloor::PersonalServiceApproval,
        ExecutionFloor::SharedConnectionConsent,
    ];
    p.validity = Validity::Persistent;
    g.expires_at = contract::legacy_expiry(&p.validity);
    g
}

#[tokio::test]
async fn phase_b_voice_task_covered_gap_and_prohibition_keep_chat_proposals_out() {
    for global in [false, true] {
        use super::tests::proposal_tests::assistant_for_cohort;
        for outcome in ["covered", "gap", "prohibited"] {
            let (f, row) = assistant_for_cohort("phase_b_voice", global).await;
            if outcome == "gap" {
                // This path must refuse before any waiting/notification setup.
                super::super::assistant_settings_service::update(
                    &f.state.db,
                    &f.owner,
                    super::super::assistant_settings_service::Update {
                        access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            }
            let request = Uuid::new_v4().to_string();
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"active_turn.voice_request_id":&request}},
                )
                .await
                .unwrap();
            let auth = f.auth().await;
            assert!(auth.assistant_chat.as_ref().unwrap().execution_voice_task);
            assert!(!proposals::supported_chat(
                auth.assistant_chat.as_ref().unwrap()
            ));
            if outcome != "gap" {
                let mut g = grant(&f.state, &auth, &f.connection, "GET", "/status").await;
                if outcome == "prohibited" {
                    g.execution.as_mut().unwrap().version = 99;
                }
                f.store(&g).await;
            }
            let (status, text) = f.call("/proxy/s/execution-fixture/status").await;
            if outcome == "covered" {
                assert_eq!(status, axum::http::StatusCode::OK, "{text}");
                f.no_prompts().await;
            } else {
                assert!(!status.is_success(), "{text}");
            }
            assert_eq!(
                f.server.received_requests().await.unwrap().len(),
                usize::from(outcome == "covered")
            );
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>("approval_requests")
                    .count_documents(doc! {"execution_proposal":{"$ne":null}})
                    .await
                    .unwrap(),
                0
            );
            if outcome == "prohibited" {
                f.no_prompts().await;
            }
            if outcome == "gap" {
                assert!(text.contains("approval_unsupported"), "{text}");
                f.no_prompts().await;
            }
            f.cleanup().await;
        }
    }
}

#[tokio::test]
async fn phase_b_legacy_voice_turn_without_task_does_not_adopt_enrollment() {
    let (f, _) = super::tests::proposal_tests::assistant("phase_b_old_voice").await;
    let auth = f.auth().await;
    let mut chat = auth.assistant_chat.as_deref().unwrap().clone();
    chat.execution_voice_task = true;
    chat.execution_task = None;
    assert!(RequestContext::enrolled(Some(VERSION), &f.owner, &f.key, Some(&chat)).is_none());
    f.cleanup().await;
}
