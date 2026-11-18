//! #2072: presentation never turns a terminal or unavailable request into an ask.
use super::*;
use crate::services::assistant_authority_tests::{fixture, orchestrator_fixture};

fn card(status: &str, expired: bool) -> AssistantAcknowledgement {
    let now = Utc::now();
    bson::from_document(doc! {
        "_id":uuid::Uuid::new_v4().to_string(), "conversation_id":"source-thread",
        "user_id":"owner", "api_key_id":"key", "kind":"action", "summary":"Write ledger",
        "status":status, "created_at":bson::DateTime::from_chrono(now),
        "expires_at":bson::DateTime::from_chrono(now + Duration::seconds(if expired {-1} else {60})),
    }).unwrap()
}

#[test]
fn invisible_card_2072_tool_result_state_matrix_and_exact_links() {
    for (stored, expired, status, code) in [
        ("pending", false, "pending", "acknowledgement_required"),
        ("pending", true, "expired", "acknowledgement_expired"),
        ("expired", true, "expired", "acknowledgement_expired"),
        ("allowed", false, "allowed", "acknowledgement_allowed"),
        ("allowed", true, "expired", "acknowledgement_expired"),
        ("denied", false, "denied", "acknowledgement_denied"),
        ("used", true, "used", "acknowledgement_used"),
        ("failed", false, "failed", "acknowledgement_unavailable"),
    ] {
        let row = card(stored, expired);
        let result = refusal_with_frontend(&row, "https://nyxid.example/");
        assert_eq!(result["status"], status);
        assert_eq!(result["code"], code);
        let enriched = crate::services::assistant_access_policy::with_frontend(
            refusal(&row),
            "https://nyxid.example/",
        );
        assert_eq!(enriched["review_url"], result["review_url"]);
        assert_eq!(
            result["review_path"],
            format!("/assistant?c=source-thread&card={}", row.id)
        );
        assert_eq!(
            result["review_url"],
            format!(
                "https://nyxid.example/assistant?c=source-thread&card={}",
                row.id
            )
        );
        assert_eq!(result.get("confirm_phrase").is_some(), status == "pending");
        assert!(
            !result["instructions"]
                .as_str()
                .unwrap()
                .contains("card is shown")
        );
    }
    let mut automatic = card("allowed", true);
    automatic.decided_by = Some("access_mode".into());
    assert_eq!(
        presentation_status(&automatic),
        "allowed",
        "Full-access receipts never expire"
    );
    let failure = unavailable();
    assert_eq!(failure["code"], "acknowledgement_unavailable");
    assert!(failure.get("acknowledgement_id").is_none());
    assert!(failure.get("review_path").is_none());
}

#[tokio::test]
async fn invisible_card_2072_expiry_history_and_invalid_retry_are_truthful() {
    let f = orchestrator_fixture("card_2072_expiry").await;
    let args = json!({"sheet":"ledger","range":"A1:B2"});
    let row = request(
        &f.state.db,
        &f.chat,
        Request {
            kind: "action",
            service: None,
            tool: Some("sheets_write"),
            arguments: Some(&args),
            summary: "Write ledger",
            platform: false,
        },
    )
    .await
    .unwrap();
    // A fresh history request (including an empty/offscreen message page) restores it.
    for _ in 0..2 {
        let rows = history_for_messages(&f.state.db, &f.owner, &f.row.id, None, &[])
            .await
            .unwrap();
        assert!(rows.iter().any(|r| r.id == row.id && r.status == "pending"));
    }
    let since = Utc::now() - Duration::minutes(1);
    f.state.db.collection::<bson::Document>(ACKS).update_one(doc!{"_id":&row.id},doc!{"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    let notes = decided_since(&f.state.db, &f.owner, &f.row.id, since)
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].status, "expired");
    assert!(decisions_note(&notes).contains("Expired confirmations cannot be allowed"));
    let rows = history(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    assert_eq!(rows[0].status, "expired");
    assert!(
        decide(&f.state.db, &f.owner, &f.row.id, &row.id, true)
            .await
            .is_err()
    );
    let need = ConfirmationNeed {
        org_instruction: OrgInstructionRequirement::None,
        class: OwnerActionClass::Change,
        mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
        webhook: false,
        policy: false,
        delegation: false,
    };
    let mut retry = args.clone();
    retry["acknowledgement_id"] = json!(&row.id);
    let result = confirm_action(
        &f.state.db,
        &f.chat,
        "sheets_write",
        &retry,
        &need,
        "Write ledger",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result["error"], "acknowledgement_invalid");
    assert_eq!(result["code"], "acknowledgement_expired");
    assert_eq!(result["status"], "expired");
    assert!(result.get("confirm_phrase").is_none());
    // Another invocation's card must not be presented as this call's authority.
    f.state
        .db
        .collection::<bson::Document>(ACKS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"invocation_id":"different-invocation"}},
        )
        .await
        .unwrap();
    let result = confirm_action(
        &f.state.db,
        &f.chat,
        "sheets_write",
        &retry,
        &need,
        "Write ledger",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result["code"], "acknowledgement_invalid");
    assert_eq!(result["status"], "unavailable");
    assert!(result.get("review_path").is_none());
}

#[tokio::test]
async fn invisible_card_2072_cross_thread_summaries_link_the_actual_source_card() {
    let f = fixture("card_2072_routed").await;
    let (result, row) = account_gate(&f.state.db, &f.chat).await.unwrap().unwrap();
    let row = row.unwrap();
    assert_ne!(f.row.id, f.nyxbot_thread);
    let summary = crate::services::assistant_team_service::request_summaries(
        &f.state.db,
        &f.owner,
        std::slice::from_ref(&row),
    )
    .await
    .unwrap();
    assert_eq!(summary[0].status, "pending");
    assert_eq!(summary[0].review_path, result["review_path"]);
    assert!(summary[0].review_path.contains(&f.row.id));
    assert!(!summary[0].review_path.contains(&f.nyxbot_thread));
    assert!(
        history(&f.state.db, &f.owner, &f.nyxbot_thread)
            .await
            .unwrap()
            .is_empty()
    );
    let mut terminal = row;
    terminal.expires_at = Utc::now() - Duration::seconds(1);
    let summary = crate::services::assistant_team_service::request_summary(&terminal, None);
    assert_eq!(summary.code, "acknowledgement_expired");
}

#[tokio::test]
async fn invisible_card_2072_failed_creation_and_guest_cannot_claim_a_waiting_card() {
    let f = orchestrator_fixture("card_2072_creation_failure").await;
    let args = json!({"sheet":"ledger"});
    f.state.db.run_command(doc!{"collMod":ACKS,"validator":{"summary":{"$ne":"reject-card"}},"validationLevel":"strict"}).await.unwrap();
    assert!(
        request(
            &f.state.db,
            &f.chat,
            Request {
                kind: "action",
                service: None,
                tool: Some("sheets_write"),
                arguments: Some(&args),
                summary: "reject-card",
                platform: false
            }
        )
        .await
        .is_err()
    );
    assert!(
        history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .is_empty()
    );
    let mut guest = f.chat.clone();
    guest.guest = true;
    let (refused, created) = service_gate(
        &f.state.db,
        &guest,
        "sheets",
        "google-sheets",
        "Google Sheets",
        false,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(refused, orchestrator_guest_refusal());
    assert!(created.is_none());
    assert!(
        history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .is_empty()
    );
}
