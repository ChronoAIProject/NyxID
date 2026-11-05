use super::*;
use crate::models::feature_flag_override::FlagTargetKind;
use crate::services::{
    assistant_nyxagent as engine, audit_service::AuditActor,
    execution_permission_enrollment as enrollment, feature_flag_admin_service as admin,
};

async fn global(f: &Fixture, enabled: bool, refresh: bool) {
    admin::mutate(
        &f.state.db,
        enrollment::FLAG,
        admin::Mutation::Override {
            kind: FlagTargetKind::Global,
            target_id: None,
            enabled: Some(enabled),
        },
        &AuditActor {
            user_id: f.owner.clone(),
            api_key_id: None,
            api_key_name: None,
            ip_address: None,
            user_agent: None,
        },
    )
    .await
    .unwrap();
    if refresh {
        f.state
            .execution_enrollment
            .refresh(&f.state.db)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn broad_stale_snapshots_choose_legacy_without_marking_fresh_work() {
    for positive in [false, true] {
        let f = Fixture::new("broad_stale_admission").await;
        global(&f, positive, true).await;
        global(&f, !positive, false).await;
        let auth = f.auth().await;
        assert_eq!(auth.execution_enrolled, positive);
        let mut target = f.target().await;
        let descriptor =
            crate::services::operation_descriptor::build_http_descriptor("GET", "status", None);
        admit_http(
            &f.state,
            &auth,
            Some(&f.connection),
            Some(&f.owner),
            &mut target,
            "GET",
            "status",
            None,
            b"",
            &Default::default(),
            None,
            &descriptor,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(target.execution_request.is_none());
        assert!(target.execution_permission.is_none());
        if let Some(context) = &auth.execution_permission {
            assert!(!context.participating());
        }
        let request = engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            agent_id: None,
            conversation_id: None,
            text: "Fresh request".into(),
            model: None,
            access_mode: None,
        };
        let mut start = engine::TurnStart::from(&request);
        start.execution_enrollment = auth.execution_enrolled;
        let row = engine::begin_turn(&f.state.db, &f.owner, start, &f.state.encryption_keys)
            .await
            .unwrap();
        assert!(row.active_turn.unwrap().execution_task.is_none());
        for (collection, field) in [
            ("assistant_messages", "execution_task"),
            ("assistant_invocations", "execution_task"),
            ("approval_requests", "execution_proposal"),
            ("machine_jobs", "execution_task"),
            ("async_service_operations", "execution_task"),
        ] {
            let mut filter = doc! {};
            filter.insert(field, doc! {"$ne":null});
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(collection)
                    .count_documents(filter)
                    .await
                    .unwrap(),
                0,
                "{collection}"
            );
        }
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[tokio::test]
async fn broad_marked_task_drains_on_live_disable_even_with_a_cold_snapshot() {
    let (f, row) = proposal_tests::assistant_for_cohort("broad_marked_drain", true).await;
    global(&f, false, true).await;
    // Auth reconstructs the context from durable lineage despite a negative hint.
    let auth = f.auth().await;
    assert!(!auth.execution_enrolled);
    assert!(auth.execution_permission.is_some());
    assert!(
        auth.assistant_chat
            .as_ref()
            .unwrap()
            .execution_task
            .is_some()
    );
    let mut target = f.target().await;
    let descriptor =
        crate::services::operation_descriptor::build_http_descriptor("GET", "status", None);
    assert!(
        admit_http(
            &f.state,
            &auth,
            Some(&f.connection),
            Some(&f.owner),
            &mut target,
            "GET",
            "status",
            None,
            b"",
            &Default::default(),
            None,
            &descriptor,
            false,
            None
        )
        .await
        .is_err()
    );
    let live = f
        .state
        .db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            "assistant_conversations",
        )
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert!(live.active_turn.unwrap().execution_task.is_some());
    f.no_prompts().await;
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}
