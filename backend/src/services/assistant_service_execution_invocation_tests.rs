use super::*;
use crate::models::machine_job::DeclaredService;
use crate::services::{
    assistant_invocation_service as invocations, assistant_job_service_use as jobs,
    assistant_nyxagent as engine, machine_gateway_service, machine_integration_tests,
    machine_service as machines,
};

async fn finish(f: &Fixture) {
    engine::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "submitted".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}

async fn invocation_fixture() -> (
    Fixture,
    crate::models::assistant_invocation::AssistantInvocation,
) {
    let mut f = orchestrator_fixture("shared_invocation_execution").await;
    finish(&f).await;
    invocations::ensure_indexes(&f.state.db).await.unwrap();
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: Some(f.row.id.clone()),
        agent_id: None,
        text: "run exact owner work".into(),
        model: None,
        access_mode: None,
    });
    start.origin = crate::models::assistant_conversation::TurnOrigin::User;
    let admission = invocations::admit(
        &f.state.db,
        &f.owner,
        &Uuid::new_v4().to_string(),
        "fixture request fingerprint",
        &start,
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    f.row = admission.claim.unwrap().conversation;
    f.chat = acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    mode(&f, AccessMode::Nyxmode).await;
    (f, admission.invocation)
}

#[tokio::test]
async fn invocation_shared_admission_rejects_root_stop_and_changed_lineage_at_final_egress() {
    for mutation in ["cancel", "delete", "terminal", "lineage"] {
        let (f, root) = invocation_fixture().await;
        let connection = connection(&f, &f.owner, "bounded", None).await;
        let node = machine_integration_tests::node(&f, &f.owner).await;
        let admission = gate(&f, &connection, &f.owner, &json!({"path":"/read"}), None)
            .await
            .admission
            .unwrap();
        admission
            .revalidate(&f.state.db, &connection, None)
            .await
            .unwrap();
        if mutation == "lineage" {
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.invocation_id":bson::Bson::Null}},
                )
                .await
                .unwrap();
        } else {
            let change = match mutation {
                "cancel" => doc! {"cancel_requested":true},
                "delete" => doc! {"conversation_deleted":true,"cancel_requested":true},
                "terminal" => doc! {"terminal_status":"completed"},
                _ => unreachable!(),
            };
            f.state
                .db
                .collection::<Document>(crate::models::assistant_invocation::COLLECTION_NAME)
                .update_one(doc! {"_id":&root.id}, doc! {"$set":change})
                .await
                .unwrap();
        }
        assert!(
            admission
                .revalidate(&f.state.db, &connection, None)
                .await
                .is_err(),
            "{mutation}"
        );
        assert!(
            machines::admit_turn_operation(&f.state.db, &f.chat, &node)
                .await
                .is_err(),
            "machine operation admission: {mutation}"
        );
        assert!(
            machines::issue_job(&f.state.db, &f.chat, &node, 120, Vec::new())
                .await
                .is_err(),
            "machine job publication: {mutation}"
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>("machine_jobs")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn invocation_settled_machine_job_checks_issuing_root_before_gateway_and_service_egress() {
    for mutation in ["cancel", "delete", "terminal", "missing"] {
        let (f, root) = invocation_fixture().await;
        let node = machine_integration_tests::node(&f, &f.owner).await;
        let connection = connection(&f, &f.owner, "job-service", None).await;
        let catalog = Uuid::new_v4().to_string();
        let job = machines::issue_job(
            &f.state.db,
            &f.chat,
            &node,
            120,
            vec![
                DeclaredService {
                    id: connection.clone(),
                    slug: "job-service".into(),
                },
                DeclaredService {
                    id: catalog.clone(),
                    slug: "job-catalog".into(),
                },
            ],
        )
        .await
        .unwrap();
        finish(&f).await;
        assert!(
            machine_gateway_service::job_auth(&f.state.db, &job)
                .await
                .is_ok(),
            "bounded job survives issuing turn settlement"
        );
        assert!(
            jobs::validate(&f.state.db, &job.id, &connection, &f.owner)
                .await
                .is_ok()
        );
        assert!(
            jobs::validate_catalog(&f.state.db, &job.id, &catalog)
                .await
                .is_ok()
        );
        let roots = f
            .state
            .db
            .collection::<Document>(crate::models::assistant_invocation::COLLECTION_NAME);
        if mutation == "missing" {
            roots.delete_one(doc! {"_id":&root.id}).await.unwrap();
        } else {
            let change = match mutation {
                "cancel" => doc! {"cancel_requested":true},
                "delete" => doc! {"conversation_deleted":true,"cancel_requested":true},
                "terminal" => doc! {"terminal_status":"completed"},
                _ => unreachable!(),
            };
            roots
                .update_one(doc! {"_id":&root.id}, doc! {"$set":change})
                .await
                .unwrap();
        }
        assert!(
            machine_gateway_service::job_auth(&f.state.db, &job)
                .await
                .is_err(),
            "gateway: {mutation}"
        );
        assert!(
            jobs::validate(&f.state.db, &job.id, &connection, &f.owner)
                .await
                .is_err(),
            "connection: {mutation}"
        );
        assert!(
            jobs::validate_catalog(&f.state.db, &job.id, &catalog)
                .await
                .is_err(),
            "catalog: {mutation}"
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>("machine_jobs")
                .find_one(doc! {"_id":&job.id})
                .await
                .unwrap()
                .unwrap()
                .get_str("state"),
            Ok("running"),
            "root fence applies before job sweep"
        );
    }
}

#[tokio::test]
async fn invocation_machine_boundary_revokes_old_key_but_allows_ordinary_successor_job() {
    let (mut f, root) = invocation_fixture().await;
    let node = machine_integration_tests::node(&f, &f.owner).await;
    let connection = connection(&f, &f.owner, "job-service", None).await;
    let declared = vec![DeclaredService {
        id: connection.clone(),
        slug: "job-service".into(),
    }];
    let old = machines::issue_job(&f.state.db, &f.chat, &node, 120, declared.clone())
        .await
        .unwrap();
    finish(&f).await;
    machines::finish(&f.state.db, &old.id).await.unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::assistant_invocation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&root.id},
            doc! {"$set":{"terminal_status":"completed"}},
        )
        .await
        .unwrap();
    f.row = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &engine::TurnStart::from(&engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            conversation_id: Some(f.row.id.clone()),
            agent_id: None,
            text: "ordinary successor".into(),
            model: None,
            access_mode: None,
        }),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    f.chat = acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(f.chat.invocation_controlled && f.chat.invocation_id.is_none());
    assert_ne!(f.chat.api_key_id, old.api_key_id);
    assert!(
        machine_gateway_service::job_auth(&f.state.db, &old)
            .await
            .is_err()
    );
    let next = machines::issue_job(&f.state.db, &f.chat, &node, 120, declared)
        .await
        .unwrap();
    assert!(
        machine_gateway_service::job_auth(&f.state.db, &next)
            .await
            .is_ok()
    );
    assert!(
        jobs::validate(&f.state.db, &next.id, &connection, &f.owner)
            .await
            .is_ok()
    );
}
