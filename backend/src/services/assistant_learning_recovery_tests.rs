//! Recovery of drafts prepared before interface snapshots (#1828), against
//! rows in their real pre-#1828 shape (fields added since then are absent).
use super::*;
use crate::models::{
    assistant_agent_learning::{
        LearningPublicationTarget, MIGRATIONS_COLLECTION_NAME, PUBLICATION_TARGETS_COLLECTION_NAME,
    },
    catalog_skill_revision::SkillReference,
};
use futures::TryStreamExt;
use std::io::Read;

const LEGACY: &str = review::LEGACY_PACKAGE;
const BASE_TOOL: &str = "search_web";
/// Fields #1828 and this follow-up added to `publication`.
const POST_1828_FIELDS: [&str; 13] = [
    "attempt",
    "package_format",
    "interface_encrypted",
    "target_kind",
    "target_skill_id",
    "dispatched_at",
    "legacy_classified_at",
    "review_card_pending",
    "release_evidence",
    "last_stage",
    "last_registry_status",
    "uncertain_dispatch",
    "verified_at",
];

/// A private tool-based Ornn skill attached to the agent at 1.0.
struct Base {
    skill_id: String,
    sha: String,
    bytes: Vec<u8>,
}

async fn tool_base(f: &Fixture, server: &MockServer) -> Base {
    let draft = GeneratedProposal {
        schema_version: 1,
        kind: "new".into(),
        name: "weekly-review".into(),
        description: "Weekly review".into(),
        skill_md: "# Weekly review".into(),
        files: vec![],
        base_skill: None,
        rationale: String::new(),
        safety_notes: String::new(),
    };
    let bytes =
        publication::package_with_snapshot(&draft, "base-operation", "weekly-review", "1.0", None)
            .unwrap();
    let base = Base {
        skill_id: Uuid::new_v4().to_string(),
        sha: publication::hash(&bytes),
        bytes,
    };
    let pin = SkillReference {
        source: "ornn".into(),
        skill_id: base.skill_id.clone(),
        name: "weekly-review".into(),
        version: "1.0".into(),
        sha256: base.sha.clone(),
        dependencies: vec![],
    };
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills":bson::to_bson(&vec![pin]).unwrap()}},
        )
        .await
        .unwrap();
    mount_base(f, server, &base).await;
    base
}

async fn mount_base(f: &Fixture, server: &MockServer, base: &Base) {
    mount_base_after(f, server, base, std::time::Duration::ZERO).await;
}

/// The base reads, each exact-version read answered after `delay`.
async fn mount_base_after(
    f: &Fixture,
    server: &MockServer,
    base: &Base,
    delay: std::time::Duration,
) {
    let id = &base.skill_id;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}")))
        .and(query_param("version", "1.0"))
        .respond_with(ResponseTemplate::new(200).set_delay(delay).set_body_json(json!({"data":{"guid":id,
            "name":"weekly-review","version":"1.0","skillHash":base.sha,"description":"Weekly review",
            "isPrivate":true,"createdBy":f.owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[],
            "metadata":{"category":"tool-based","tools":[{"tool":BASE_TOOL}]}}})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/closure")))
        .and(query_param("version", "1.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions/1.0/download")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(base.bytes.clone()))
        .mount(server)
        .await;
    mount_latest(server, base, "1.0").await;
}

async fn mount_latest(server: &MockServer, base: &Base, latest: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{}/versions", base.skill_id)))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"items":[{"version":latest}]}})),
        )
        .mount(server)
        .await;
}

/// The exact revision version (1.1) as Ornn reports it.
async fn mount_exact(server: &MockServer, base: &Base, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{}", base.skill_id)))
        .and(query_param("version", "1.1"))
        .respond_with(response)
        .mount(server)
        .await;
}

fn version_not_found() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({"code":"skill_version_not_found","status":404}))
}

async fn decode(f: &Fixture, encrypted: &[u8]) -> Vec<u8> {
    f.state.encryption_keys.decrypt(encrypted).await.unwrap()
}

async fn legacy_bytes(f: &Fixture, row: &AssistantAgentLearningProposal) -> Vec<u8> {
    let p = row.publication.as_ref().unwrap();
    let draft: GeneratedProposal =
        serde_json::from_slice(&decode(f, &row.body_encrypted).await).unwrap();
    publication::package_with_snapshot(&draft, &p.operation_id, &p.name, &p.version, None).unwrap()
}

/// An improvement of the tool-based base, drafted by NyxBot and then turned
/// into the row and card a pre-#1828 server stored: a `category: plain`
/// package without any field added since.
async fn legacy_draft(
    f: &Fixture,
    base: &Base,
    step: &str,
) -> (AssistantAgentLearningProposal, AssistantAcknowledgement) {
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":base.skill_id,"version":"1.0"});
    revision["skill_md"] = format!("# Weekly review\n1. {step}").into();
    let (row, card) = make_with_access(f, revision, true).await;
    let sha = publication::hash(&legacy_bytes(f, &row).await);
    let mut unset = bson::Document::new();
    for field in POST_1828_FIELDS {
        unset.insert(format!("publication.{field}"), "");
    }
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.sha256":&sha},"$unset":unset},
        )
        .await
        .unwrap();
    assert_pre_1828_shape(f, &row.id).await;
    let row = stored(f, &row.id).await;
    let reference = card.authored_skill.as_ref().unwrap();
    let digest = acks::arguments_digest(&publication::binding(
        &row,
        row.publication.as_ref().unwrap(),
        reference.skills_revision,
    ));
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$set":{"arguments_digest":digest}},
        )
        .await
        .unwrap();
    let card = acknowledgement(f, &card.id).await;
    (row, card)
}

async fn assert_pre_1828_shape(f: &Fixture, id: &str) {
    let raw = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    let p = raw.get_document("publication").unwrap();
    for field in POST_1828_FIELDS {
        assert!(!p.contains_key(field), "{field} must be absent");
    }
}

async fn acknowledgement(f: &Fixture, id: &str) -> AssistantAcknowledgement {
    f.state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

async fn expire(f: &Fixture, card: &AssistantAcknowledgement) -> AssistantAcknowledgement {
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$set":{"status":"expired",
            "expires_at":bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}},
        )
        .await
        .unwrap();
    acknowledgement(f, &card.id).await
}

/// A pre-#1828 attempt that stalled after dispatch, then the deploy: the
/// startup migration classifies it, holds its target and stamps it.
async fn dispatch_and_deploy(
    f: &Fixture,
    row: &AssistantAgentLearningProposal,
    card: &AssistantAcknowledgement,
) -> AssistantAgentLearningProposal {
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(doc! {"_id":&card.id}, doc! {"$set":{"status":"used"}})
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"status":"publication_failed",
            "failure_code":"publication_retry_required","publication.started":true,
            "publication.approved_by":&f.owner,"publication.acknowledgement_id":&card.id,
            "publication.approval_digest":card.arguments_digest.as_ref().unwrap(),
            "publication.lease_id":"pre-1828-lease",
            "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(30))}})
        .await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert_eq!(
        review::migrate_publication_targets(&f.state, false)
            .await
            .unwrap(),
        review::MigrationOutcome::Complete
    );
    stored(f, &row.id).await
}

async fn age(f: &Fixture, row: &AssistantAgentLearningProposal, hours: i64) {
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.legacy_classified_at":
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(hours))}},
        )
        .await
        .unwrap();
}

async fn reprepare(
    f: &Fixture,
    row: &AssistantAgentLearningProposal,
    card: Option<&AssistantAcknowledgement>,
) -> AppResult<Value> {
    let body = match card {
        Some(card) => json!({"acknowledgement_id":card.id}),
        None => json!({}),
    };
    crate::handlers::assistant_agent_learning::reprepare(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(serde_json::from_value(body).unwrap()),
    )
    .await
    .map(|axum::Json(value)| value)
}

async fn admin(f: &Fixture) -> crate::mw::auth::AuthUser {
    crate::services::role_service::seed_system_roles(&f.state.db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
        .await
        .unwrap();
    let id = Uuid::new_v4().to_string();
    let mut user = crate::test_utils::test_user(&id, crate::models::user::UserType::Person);
    user.role_ids = vec![roles.admin];
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(user)
        .await
        .unwrap();
    crate::test_utils::test_auth_user(&id)
}

async fn release_stale(
    f: &Fixture,
    auth: crate::mw::auth::AuthUser,
    body: Value,
) -> AppResult<Value> {
    crate::handlers::assistant_agent_learning::admin_release_stale(
        axum::extract::State(f.state.clone()),
        auth,
        axum::Json(serde_json::from_value(body).unwrap()),
    )
    .await
    .map(|axum::Json(value)| value)
}

fn decisions(result: &Value) -> Vec<String> {
    result["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["decision"].as_str().unwrap().to_owned())
        .collect()
}

async fn writes(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| matches!(r.method.as_str(), "PUT" | "POST"))
        .count()
}

/// Every Ornn request carried the owner's signed identity.
async fn assert_owner_identity(f: &Fixture, server: &MockServer) {
    for request in server.received_requests().await.unwrap() {
        let token = request
            .headers
            .get("x-nyxid-identity-token")
            .unwrap()
            .to_str()
            .unwrap();
        let claims: Value = serde_json::from_slice(
            &base64::Engine::decode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                token.split('.').nth(1).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["sub"], f.owner);
    }
}

async fn audits(f: &Fixture, event: &str, proposal: &str) -> Vec<bson::Document> {
    f.state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type":event,"event_data.proposal_id":proposal})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn barrier(f: &Fixture, base: &Base) -> Option<LearningPublicationTarget> {
    f.state
        .db
        .collection::<LearningPublicationTarget>(PUBLICATION_TARGETS_COLLECTION_NAME)
        .find_one(doc! {"_id":format!("{}:1.1", base.skill_id)})
        .await
        .unwrap()
}

/// The rebuilt card raised after a rebuild: pending, same conversation.
async fn rebuilt_card(
    f: &Fixture,
    row: &AssistantAgentLearningProposal,
) -> AssistantAcknowledgement {
    let row = stored(f, &row.id).await;
    let digest = acks::arguments_digest(&publication::binding(
        &row,
        row.publication.as_ref().unwrap(),
        row.publication.as_ref().unwrap().skills_revision,
    ));
    f.state
        .db
        .collection::<AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"authored_skill.proposal_id":&row.id,"arguments_digest":digest})
        .await
        .unwrap()
        .unwrap()
}

async fn put_skill_md(server: &MockServer) -> String {
    let put = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method.as_str() == "PUT")
        .unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(put.body)).unwrap();
    let mut markdown = String::new();
    archive
        .by_name("weekly-review/SKILL.md")
        .unwrap()
        .read_to_string(&mut markdown)
        .unwrap();
    markdown
}

/// Publishes the current rebuilt package from its card and returns the
/// SKILL.md Ornn received.
async fn publish_rebuilt(
    f: &Fixture,
    server: &MockServer,
    base: &Base,
    row: &AssistantAgentLearningProposal,
) -> String {
    server.reset().await;
    mount_base(f, server, base).await;
    let rebuilt = stored(f, &row.id).await;
    mount_publication(f, server, &rebuilt, &base.skill_id, "PUT").await;
    let card = rebuilt_card(f, &rebuilt).await;
    assert_eq!(finish(f, card, true).await.unwrap().status, "used");
    server.verify().await;
    let pinned = stored(f, &row.id).await;
    assert_eq!(pinned.status, "pinned");
    let agent =
        crate::services::assistant_team_service::agent(&f.state.db, &f.owner, &row.agent_id)
            .await
            .unwrap();
    assert!(agent.skills.iter().any(|pin| pin.skill_id == base.skill_id
        && pin.version == "1.1"
        && pin.sha256 == rebuilt.publication.as_ref().unwrap().sha256));
    put_skill_md(server).await
}

fn assert_tools_preserved(markdown: &str) {
    assert!(markdown.contains("category: \"tool-based\""), "{markdown}");
    assert!(
        markdown.contains(&format!("tool-list: [\"{BASE_TOOL}\"]")),
        "{markdown}"
    );
}

#[tokio::test]
async fn recovery_expired_legacy_card_rebuilds_in_its_conversation_and_publishes_tools() {
    let f = fixture("recovery_rebuild_expired_card").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (0).").await;
    let card = expire(&f, &card).await;
    let old = row.publication.clone().unwrap();
    assert!(
        legacy_bytes(&f, &row)
            .await
            .windows(16)
            .any(|w| w == b"category: plain\n")
    );

    // Only the rebuild (or discarding) remains; nothing can dispatch it.
    let preview = card_preview(&f, &row, &card).await;
    assert_eq!(preview["actions"], json!(["reprepare", "discard"]));
    assert_eq!(preview["failure_code"], LEGACY);
    let listed = review::list(&f.state, &f.owner, &row.agent_id, true)
        .await
        .unwrap();
    assert_eq!(listed[0].failure_code.as_deref(), Some(LEGACY));
    assert!(matches!(finish(&f, card.clone(), true).await,
        Err(AppError::Conflict(message)) if message == WITHHELD));
    let reader = crate::handlers::agent_skills::Reader {
        state: &f.state,
        person: &f.owner,
        thread_key: None,
        scopes: None,
        chat: None,
    };
    assert!(matches!(
        review::approval_binding(&f.state, &f.owner, &row.agent_id, &row.id, row.revision,
            card.authored_skill.as_ref().unwrap().skills_revision, &reader).await,
        Err(AppError::Conflict(message)) if message == LEGACY));
    let binding = publication::binding(
        &row,
        &old,
        card.authored_skill.as_ref().unwrap().skills_revision,
    );
    assert!(matches!(
        review::approve(&f.state, &f.chat, &row.agent_id, &row.id, &card.id, &binding, &reader).await,
        Err(AppError::Conflict(message)) if message == LEGACY));
    let renewal = crate::handlers::assistant_agent_learning::approve(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((row.agent_id.clone(), row.id.clone())),
        axum::Json(
            serde_json::from_value(json!({"revision":row.revision,
                "agent_skills_revision":card.authored_skill.as_ref().unwrap().skills_revision,
                "renewal_of":card.id}))
            .unwrap(),
        ),
    )
    .await;
    assert!(matches!(renewal, Err(AppError::Conflict(message)) if message == LEGACY));
    // An authored draft is rebuilt only from its card.
    assert!(matches!(
        reprepare(&f, &row, None).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(writes(&server).await, 0);

    let result = reprepare(&f, &row, Some(&card)).await.unwrap();
    assert_eq!(result["status"], "reprepared");
    let rebuilt = stored(&f, &row.id).await;
    let p = rebuilt.publication.clone().unwrap();
    assert_eq!(rebuilt.revision, row.revision + 1);
    assert_eq!(rebuilt.status, "pending");
    assert_eq!(rebuilt.failure_code, None);
    assert_ne!(p.operation_id, old.operation_id);
    assert_ne!(p.sha256, old.sha256);
    assert_eq!(
        (p.name.as_str(), p.version.as_str()),
        ("weekly-review", "1.1")
    );
    assert_eq!(p.package_format, 2);
    assert!(p.interface_encrypted.is_some() && !p.review_card_pending);
    assert_eq!(p.target_kind.as_deref(), Some("update"));
    assert_eq!(p.target_skill_id.as_deref(), Some(base.skill_id.as_str()));
    assert!(!p.started && p.attempt == 0);
    // The new card is in the original conversation, for the new package.
    let new_card = rebuilt_card(&f, &row).await;
    assert_eq!(result["acknowledgement_id"], new_card.id);
    assert_eq!(new_card.status, "pending");
    assert_eq!(new_card.conversation_id, card.conversation_id);
    assert_eq!(new_card.api_key_id, card.api_key_id);
    assert_eq!(
        new_card.authored_skill.as_ref().unwrap().revision,
        row.revision + 1
    );
    let old_preview = card_preview(&f, &rebuilt, &card).await;
    assert_eq!(old_preview["state"], "changed");
    assert_eq!(old_preview["actions"], json!([]));
    let new_preview = card_preview(&f, &rebuilt, &new_card).await;
    assert_eq!(new_preview["actions"], json!(["deny", "publish"]));
    assert!(
        new_preview["files"][0]["content"]
            .as_str()
            .unwrap()
            .contains("category: \"tool-based\"")
    );
    let audit = audits(&f, "assistant_learning_proposal_reprepared", &row.id).await;
    assert_eq!(audit.len(), 1);
    let data = audit[0].get_document("event_data").unwrap();
    assert_eq!(
        data.get_str("previous_operation_id").unwrap(),
        old.operation_id
    );
    assert_eq!(data.get_str("operation_id").unwrap(), p.operation_id);
    assert_eq!(data.get_str("acknowledgement_id").unwrap(), card.id);
    // The base was read with the owner's identity; nothing was written.
    assert_eq!(writes(&server).await, 0);
    assert_owner_identity(&f, &server).await;
    // A repeated click finds nothing left to do.
    assert!(reprepare(&f, &row, Some(&card)).await.is_err());

    assert_tools_preserved(&publish_rebuilt(&f, &server, &base, &row).await);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_interrupted_rebuild_raises_exactly_one_card_from_old_cards() {
    let f = fixture("recovery_rebuild_phase_two").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (1).").await;
    reprepare(&f, &row, Some(&card)).await.unwrap();
    // Crash after the swap committed and before its card existed.
    let new_card = rebuilt_card(&f, &row).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .delete_one(doc! {"_id":&new_card.id})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.review_card_pending":true}},
        )
        .await
        .unwrap();
    let rebuilt = stored(&f, &row.id).await;
    let old_preview = card_preview(&f, &rebuilt, &card).await;
    assert_eq!(old_preview["state"], "changed");
    assert_eq!(old_preview["actions"], json!(["show_updated_draft"]));
    // An expired old card works too, and concurrent clicks raise one card.
    let expired = expire(&f, &card).await;
    let (first, second) = tokio::join!(
        reprepare(&f, &row, Some(&card)),
        reprepare(&f, &row, Some(&expired))
    );
    let raised: Vec<_> = [first, second].into_iter().filter_map(Result::ok).collect();
    assert!(!raised.is_empty());
    let digest = acks::arguments_digest(&publication::binding(
        &rebuilt,
        rebuilt.publication.as_ref().unwrap(),
        rebuilt.publication.as_ref().unwrap().skills_revision,
    ));
    let cards = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .count_documents(doc! {"authored_skill.proposal_id":&row.id,"arguments_digest":&digest})
        .await
        .unwrap();
    assert_eq!(cards, 1);
    for result in &raised {
        assert_eq!(
            result["acknowledgement_id"],
            raised[0]["acknowledgement_id"]
        );
    }
    let settled = stored(&f, &row.id).await;
    assert!(!settled.publication.as_ref().unwrap().review_card_pending);
    // The swap did not repeat, and nothing else is offered on the old card.
    assert_eq!(
        settled.publication.as_ref().unwrap().operation_id,
        rebuilt.publication.as_ref().unwrap().operation_id
    );
    assert_eq!(
        card_preview(&f, &settled, &card).await["actions"],
        json!([])
    );
    assert!(reprepare(&f, &row, Some(&card)).await.is_err());
    assert_eq!(
        audits(&f, "assistant_learning_proposal_reprepared", &row.id)
            .await
            .len(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_concurrent_rebuild_renewal_and_denial_never_both_take_effect() {
    let f = fixture("recovery_rebuild_races").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (2).").await;
    let old = row.publication.clone().unwrap();
    let skills_revision = card.authored_skill.as_ref().unwrap().skills_revision;
    let renew = || {
        crate::handlers::assistant_agent_learning::approve(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&f.owner),
            axum::extract::Path((row.agent_id.clone(), row.id.clone())),
            axum::Json(
                serde_json::from_value(json!({"revision":row.revision,
                    "agent_skills_revision":skills_revision,"renewal_of":card.id}))
                .unwrap(),
            ),
        )
    };
    let (first, second, renewal, denial) = tokio::join!(
        reprepare(&f, &row, Some(&card)),
        reprepare(&f, &row, Some(&card)),
        renew(),
        finish(&f, card.clone(), false)
    );
    assert!(renewal.is_err());
    let after = stored(&f, &row.id).await;
    let p = after.publication.clone().unwrap();
    let rebuilds = audits(&f, "assistant_learning_proposal_reprepared", &row.id)
        .await
        .len();
    let decided = acknowledgement(&f, &card.id).await;
    if after.status == "rejected" {
        // The denial won: the legacy operation was discarded, never rebuilt.
        assert_eq!(p.operation_id, old.operation_id);
        assert_eq!(rebuilds, 0);
        assert!(first.is_err() && second.is_err());
        assert_eq!(denial.unwrap().status, "denied");
        assert_eq!(decided.status, "denied");
    } else {
        assert_eq!(after.status, "pending");
        assert_ne!(p.operation_id, old.operation_id);
        assert_eq!((p.package_format, rebuilds), (2, 1));
        assert_eq!(after.revision, row.revision + 1);
        let digest = acks::arguments_digest(&publication::binding(&after, &p, p.skills_revision));
        let cards = f
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {"authored_skill.proposal_id":&row.id,"arguments_digest":&digest})
            .await
            .unwrap();
        // Exactly one card for the rebuilt package, and none still pending.
        assert_eq!((cards, p.review_card_pending), (1, false));
        assert!(first.is_ok() || second.is_ok());
        // A denial that ran before the swap only dismissed the old card;
        // after it, the old card no longer offered Deny.
        match denial {
            Ok(denied) => {
                assert_eq!(denied.status, "denied");
                assert_eq!(decided.status, "denied");
            }
            Err(AppError::Conflict(message)) => {
                assert_eq!(message, WITHHELD);
                assert_ne!(decided.status, "denied");
            }
            Err(error) => panic!("unexpected denial outcome: {error:?}"),
        }
    }
    assert_eq!(writes(&server).await, 0);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_rebuild_is_refused_for_snapshot_dispatched_leased_and_changed_drafts() {
    let f = fixture("recovery_rebuild_eligibility").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    // A snapshot-aware draft is never rebuilt.
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":base.skill_id,"version":"1.0"});
    let (current, current_card) = make_with_access(&f, revision, true).await;
    assert_eq!(current.publication.as_ref().unwrap().package_format, 2);
    assert!(
        !card_preview(&f, &current, &current_card).await["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("reprepare"))
    );
    assert!(reprepare(&f, &current, Some(&current_card)).await.is_err());
    review::reject(
        &f.state,
        &f.owner,
        &current.agent_id,
        &current.id,
        current.revision,
        "rejected",
    )
    .await
    .unwrap();

    let (row, card) = legacy_draft(&f, &base, "Search the tracker (3).").await;
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    // A live lease (an attempt in flight) refuses the rebuild.
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.lease_expires_at":
            bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(60))}},
        )
        .await
        .unwrap();
    assert!(
        !card_preview(&f, &row, &card).await["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("reprepare"))
    );
    assert!(reprepare(&f, &row, Some(&card)).await.is_err());
    // A dispatched legacy attempt is only checked.
    proposals
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"publication.started":true,
            "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}})
        .await
        .unwrap();
    assert!(
        !card_preview(&f, &row, &card).await["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("reprepare"))
    );
    assert!(reprepare(&f, &row, Some(&card)).await.is_err());
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.started":false}},
        )
        .await
        .unwrap();
    // A card of an older revision of the draft is changed and cannot rebuild.
    let stale = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME);
    stale
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$inc":{"authored_skill.revision":-1_i64}},
        )
        .await
        .unwrap();
    let older = acknowledgement(&f, &card.id).await;
    assert_eq!(card_preview(&f, &row, &older).await["state"], "changed");
    assert!(reprepare(&f, &row, Some(&older)).await.is_err());
    stale
        .update_one(
            doc! {"_id":&card.id},
            doc! {"$inc":{"authored_skill.revision":1_i64}},
        )
        .await
        .unwrap();
    // A base that moved on cannot be reproduced: the card says so.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.sha256":"f".repeat(64)}},
        )
        .await
        .unwrap();
    let preview = card_preview(&f, &row, &card).await;
    assert!(
        !preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("reprepare"))
    );
    assert_eq!(preview["failure_code"], "base_changed");
    assert!(reprepare(&f, &row, Some(&card)).await.is_err());
    let unchanged = stored(&f, &row.id).await;
    assert_eq!(
        unchanged.publication.as_ref().unwrap().operation_id,
        row.publication.as_ref().unwrap().operation_id
    );
    assert_eq!(writes(&server).await, 0);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_rebuild_records_a_base_interface_that_cannot_be_reproduced() {
    let f = fixture("recovery_rebuild_incompatible_base").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (4).").await;
    server.reset().await;
    // The base became runtime-based without the output type Ornn requires.
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{}", base.skill_id)))
        .and(query_param("version", "1.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":base.skill_id,
            "name":"weekly-review","version":"1.0","skillHash":base.sha,"description":"Weekly review",
            "isPrivate":true,"createdBy":f.owner,"sharedWithUsers":[],"sharedWithOrgs":[],"grants":[],
            "metadata":{"category":"runtime-based","runtimes":[{"runtime":"node"}]}}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{}/closure", base.skill_id)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v1/skills/{}/versions/1.0/download",
            base.skill_id
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(base.bytes.clone()))
        .mount(&server)
        .await;
    mount_latest(&server, &base, "1.0").await;
    assert!(matches!(reprepare(&f, &row, Some(&card)).await,
        Err(AppError::Conflict(message)) if message == "base_interface_incompatible"));
    let after = stored(&f, &row.id).await;
    assert_eq!(
        after.failure_code.as_deref(),
        Some("base_interface_incompatible")
    );
    assert_eq!(
        after.publication.as_ref().unwrap().operation_id,
        row.publication.as_ref().unwrap().operation_id
    );
    // The card stops offering a rebuild that cannot succeed.
    let preview = card_preview(&f, &after, &card).await;
    assert!(
        !preview["actions"]
            .as_array()
            .unwrap()
            .contains(&json!("reprepare"))
    );
    assert_eq!(preview["failure_code"], "base_interface_incompatible");
    // A transient read failure records nothing.
    server.reset().await;
    mount_base(&f, &server, &base).await;
    let (row2, card2) = legacy_draft(&f, &base, "Search the tracker (5).").await;
    server.reset().await;
    assert!(matches!(reprepare(&f, &row2, Some(&card2)).await,
        Err(AppError::Conflict(message)) if message.starts_with("source_skill_unavailable:")));
    assert_eq!(stored(&f, &row2.id).await.failure_code, None);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_stale_release_then_rebuild_publishes_against_the_tool_base() {
    let f = fixture("recovery_release_rebuild_publish").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (6).").await;
    let row = dispatch_and_deploy(&f, &row, &card).await;
    let p = row.publication.clone().unwrap();
    assert_eq!(p.target_kind.as_deref(), Some("update"));
    assert_eq!(p.target_skill_id.as_deref(), Some(base.skill_id.as_str()));
    assert!(p.started && p.uncertain_dispatch && p.attempt == 0);
    assert!(p.legacy_classified_at.is_some() && p.dispatched_at.is_none());
    assert_eq!(barrier(&f, &base).await.unwrap().state, "uncertain");
    let used = acknowledgement(&f, &card.id).await;
    let dispatched = card_preview(&f, &row, &used).await;
    assert_eq!(dispatched["actions"], json!(["check"]));
    // The pre-#1828 retry code reads as an unknown outcome.
    assert_eq!(
        row.failure_code.as_deref(),
        Some("publication_retry_required")
    );
    assert_eq!(dispatched["failure_code"], "publish_uncertain");
    let listed = review::list(&f.state, &f.owner, &row.agent_id, true)
        .await
        .unwrap();
    assert_eq!(listed[0].failure_code.as_deref(), Some("publish_uncertain"));
    // So does NyxBot's tool, which lists proposals without their drafts.
    let (tool, _) = crate::handlers::agent_skills::dispatch(
        &f.state,
        &f.chat,
        "learning_list_proposals",
        &json!({}),
    )
    .await
    .unwrap();
    let proposals = tool["proposals"].as_array().unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0]["id"], row.id);
    assert!(proposals[0]["draft"].is_null());
    assert_eq!(proposals[0]["failure_code"], "publish_uncertain");
    assert_eq!(proposals[0]["status"], "publication_failed");
    // Too recent: the stamp is the deploy, not the original dispatch.
    let operator = admin(&f).await;
    server.reset().await;
    mount_exact(&server, &base, version_not_found()).await;
    mount_latest(&server, &base, "1.0").await;
    let fresh = release_stale(&f, operator.clone(), json!({}))
        .await
        .unwrap();
    assert_eq!(fresh["rows"], json!([]));
    age(&f, &row, 25).await;

    // Owners, API keys and delegated callers cannot release.
    assert!(matches!(
        release_stale(&f, crate::test_utils::test_auth_user(&f.owner), json!({})).await,
        Err(AppError::Forbidden(_))
    ));
    let mut keyed = operator.clone();
    keyed.api_key_id = Some("key".into());
    assert!(
        release_stale(&f, keyed, json!({"dry_run":false}))
            .await
            .is_err()
    );
    let mut delegated = operator.clone();
    delegated.auth_method = crate::mw::auth::AuthMethod::Delegated;
    assert!(
        release_stale(&f, delegated, json!({"dry_run":false}))
            .await
            .is_err()
    );

    // The default is a dry run that changes nothing.
    let dry = release_stale(&f, operator.clone(), json!({}))
        .await
        .unwrap();
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["evidence"], "absence_only");
    assert!(dry["residual"].as_str().unwrap().contains("can still land"));
    assert_eq!(dry["rows"][0]["proposal_id"], row.id);
    assert_eq!(dry["rows"][0]["decision"], "release");
    assert_eq!(dry["rows"][0]["evidence"], "absence_only");
    assert_eq!(dry["rows"][0]["target"], format!("{}:1.1", base.skill_id));
    assert!(stored(&f, &row.id).await.publication.unwrap().started);
    assert!(barrier(&f, &base).await.is_some());
    assert!(
        audits(
            &f,
            "assistant_learning_publication_target_released",
            &row.id
        )
        .await
        .is_empty()
    );

    let applied = release_stale(&f, operator.clone(), json!({"dry_run":false}))
        .await
        .unwrap();
    assert_eq!(decisions(&applied), ["released_audited"]);
    let released = stored(&f, &row.id).await;
    let released_p = released.publication.clone().unwrap();
    assert_eq!(released.status, "publication_failed");
    assert_eq!(released.failure_code.as_deref(), Some("operator_released"));
    assert!(!released_p.started && !released_p.uncertain_dispatch);
    assert_eq!(released_p.operation_id, p.operation_id);
    let evidence = released_p.release_evidence.unwrap();
    assert_eq!(
        (
            evidence.exact_version_status,
            evidence.latest_version.as_str(),
            evidence.min_age_hours
        ),
        (404, "1.0", 24)
    );
    assert_eq!(evidence.package_sha256, p.sha256);
    assert_eq!(evidence.batch_id, applied["batch_id"].as_str().unwrap());
    assert!(barrier(&f, &base).await.is_none());
    let audit = audits(
        &f,
        "assistant_learning_publication_target_released",
        &row.id,
    )
    .await;
    assert_eq!(audit.len(), 1);
    assert_eq!(
        audit[0].get_str("user_id").unwrap(),
        operator.user_id.to_string()
    );
    let data = audit[0].get_document("event_data").unwrap();
    assert_eq!(data.get_str("evidence").unwrap(), "absence_only");
    assert_eq!(data.get_str("previous_state").unwrap(), "uncertain");
    assert_eq!(data.get_str("latest_version").unwrap(), "1.0");
    assert_eq!(data.get_str("package_sha256").unwrap(), p.sha256);
    assert!(
        data.get_str("evidence_ref")
            .unwrap()
            .starts_with("bulk-stale:24h:")
    );
    let summary = f
        .state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc! {"event_type":"assistant_learning_publication_stale_release_summary"})
        .await
        .unwrap();
    assert_eq!(summary, 3);
    assert_owner_identity(&f, &server).await;
    assert_eq!(writes(&server).await, 0);
    // Released rows are no longer candidates; the old package still cannot
    // dispatch, only be rebuilt or discarded.
    let again = release_stale(&f, operator, json!({"dry_run":false}))
        .await
        .unwrap();
    assert_eq!(again["rows"], json!([]));
    let preview = card_preview(&f, &released, &used).await;
    assert_eq!(preview["actions"], json!(["reprepare", "discard"]));
    assert_eq!(preview["failure_code"], LEGACY);
    assert!(matches!(finish(&f, used.clone(), true).await,
        Err(AppError::Conflict(message)) if message == WITHHELD));

    server.reset().await;
    mount_base(&f, &server, &base).await;
    reprepare(&f, &released, Some(&used)).await.unwrap();
    assert_tools_preserved(&publish_rebuilt(&f, &server, &base, &row).await);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_stale_release_withholds_landed_uncertain_mismatched_young_and_unstamped_rows() {
    let f = fixture("recovery_release_decisions").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (7).").await;
    let row = dispatch_and_deploy(&f, &row, &card).await;
    age(&f, &row, 25).await;
    let operator = admin(&f).await;
    let apply = json!({"dry_run":false});
    for (exact, latest, expected) in [
        (
            ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":base.skill_id}})),
            "1.0",
            "landed_check_again",
        ),
        (ResponseTemplate::new(500), "1.0", "skip:uncertain"),
        (
            ResponseTemplate::new(404).set_body_json(json!({"code":"skill_not_found"})),
            "1.0",
            "skip:uncertain",
        ),
        (version_not_found(), "2.0", "skip:latest_mismatch"),
        (version_not_found(), "1.1", "skip:latest_mismatch"),
    ] {
        server.reset().await;
        mount_exact(&server, &base, exact).await;
        mount_latest(&server, &base, latest).await;
        let result = release_stale(&f, operator.clone(), apply.clone())
            .await
            .unwrap();
        assert_eq!(decisions(&result), [expected]);
        let unchanged = stored(&f, &row.id).await;
        assert!(unchanged.publication.as_ref().unwrap().started);
        assert!(
            unchanged
                .publication
                .as_ref()
                .unwrap()
                .release_evidence
                .is_none()
        );
        assert!(barrier(&f, &base).await.is_some());
    }
    assert!(
        audits(
            &f,
            "assistant_learning_publication_target_released",
            &row.id
        )
        .await
        .is_empty()
    );
    server.reset().await;
    mount_exact(&server, &base, version_not_found()).await;
    mount_latest(&server, &base, "1.0").await;
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    // Age is the latest of stamp, first dispatch and lease expiry.
    for (field, hours) in [("dispatched_at", 1), ("lease_expires_at", 1)] {
        proposals
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{format!("publication.{field}"):
                bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(hours))}},
            )
            .await
            .unwrap();
        assert_eq!(
            release_stale(&f, operator.clone(), apply.clone())
                .await
                .unwrap()["rows"],
            json!([])
        );
        proposals
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{format!("publication.{field}"):
                bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(30))}},
            )
            .await
            .unwrap();
    }
    assert_eq!(
        release_stale(&f, operator.clone(), json!({"min_age_hours":48}))
            .await
            .unwrap()["rows"],
        json!([])
    );
    // A row the startup stamp missed is reported, never released.
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$unset":{"publication.legacy_classified_at":""}},
        )
        .await
        .unwrap();
    assert_eq!(
        decisions(
            &release_stale(&f, operator.clone(), apply.clone())
                .await
                .unwrap()
        ),
        ["skip:age_unknown"]
    );
    age(&f, &row, 25).await;
    // Bounds.
    for body in [
        json!({"min_age_hours":23}),
        json!({"min_age_hours":8761}),
        json!({"limit":0}),
        json!({"limit":201}),
    ] {
        assert!(matches!(
            release_stale(&f, operator.clone(), body).await,
            Err(AppError::ValidationError(_))
        ));
    }
    // Deterministic order (oldest stamp first) and the row limit.
    let mut older = proposals
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    let older_id = Uuid::new_v4().to_string();
    older.insert("_id", &older_id);
    let copy = older.get_document_mut("publication").unwrap();
    copy.insert("operation_id", Uuid::new_v4().to_string());
    copy.insert(
        "legacy_classified_at",
        bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(40)),
    );
    let mut unstamped = older.clone();
    proposals.insert_one(older).await.unwrap();
    // An unstamped row never takes the place of a releasable one.
    let unstamped_id = Uuid::new_v4().to_string();
    unstamped.insert("_id", &unstamped_id);
    let copy = unstamped.get_document_mut("publication").unwrap();
    copy.insert("operation_id", Uuid::new_v4().to_string());
    copy.remove("legacy_classified_at");
    proposals.insert_one(unstamped).await.unwrap();
    let one = release_stale(&f, operator.clone(), json!({"limit":1}))
        .await
        .unwrap();
    assert_eq!(one["rows"].as_array().unwrap().len(), 1);
    assert_eq!(one["rows"][0]["proposal_id"], older_id);
    let two = release_stale(&f, operator.clone(), json!({"limit":2}))
        .await
        .unwrap();
    assert_eq!(two["rows"][0]["proposal_id"], older_id);
    assert_eq!(two["rows"][1]["proposal_id"], row.id);
    let all = release_stale(&f, operator, json!({"limit":3}))
        .await
        .unwrap();
    assert_eq!(all["rows"][2]["proposal_id"], unstamped_id);
    // The copies do not hold the target, so they are never released.
    assert_eq!(
        decisions(&all),
        ["skip:barrier_changed", "release", "skip:age_unknown"]
    );
    assert_eq!(writes(&server).await, 0);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_startup_backfills_format_and_stamps_dispatched_legacy_rows_once() {
    let f = fixture("recovery_startup_stamping").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    let migrations = f
        .state
        .db
        .collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME);
    // A #1828 row written before `package_format` existed has a snapshot.
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":base.skill_id,"version":"1.0"});
    let (snapshot_row, _) = make_with_access(&f, revision, true).await;
    proposals
        .update_one(
            doc! {"_id":&snapshot_row.id},
            doc! {"$unset":{"publication.package_format":""}},
        )
        .await
        .unwrap();
    migrations
        .delete_one(doc! {"_id":"publication-package-format-v1"})
        .await
        .unwrap();
    review::migrate_publication_targets(&f.state, false)
        .await
        .unwrap();
    assert_eq!(
        stored(&f, &snapshot_row.id)
            .await
            .publication
            .unwrap()
            .package_format,
        2
    );
    review::reject(
        &f.state,
        &f.owner,
        &snapshot_row.agent_id,
        &snapshot_row.id,
        snapshot_row.revision,
        "rejected",
    )
    .await
    .unwrap();

    // With both markers present, a dispatched legacy row is still stamped.
    let (row, _) = legacy_draft(&f, &base, "Search the tracker (8).").await;
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.started":true,
            "publication.target_kind":"update","publication.target_skill_id":&base.skill_id}},
        )
        .await
        .unwrap();
    assert_eq!(migrations.count_documents(doc! {}).await.unwrap(), 2);
    review::migrate_publication_targets(&f.state, false)
        .await
        .unwrap();
    let stamped = stored(&f, &row.id)
        .await
        .publication
        .unwrap()
        .legacy_classified_at
        .unwrap();
    // Set once: a later start keeps the first stamp.
    review::migrate_publication_targets(&f.state, false)
        .await
        .unwrap();
    assert_eq!(
        stored(&f, &row.id)
            .await
            .publication
            .unwrap()
            .legacy_classified_at,
        Some(stamped)
    );
    // Never stamped: undispatched, verified, or snapshot-aware rows.
    let (pending, _) = legacy_draft(&f, &base, "Search the tracker (9).").await;
    review::migrate_publication_targets(&f.state, false)
        .await
        .unwrap();
    assert!(
        stored(&f, &pending.id)
            .await
            .publication
            .unwrap()
            .legacy_classified_at
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_pre_1828_rows_accept_operator_release_and_pre_claim_records() {
    let f = fixture("recovery_absent_attempt_fences").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    // An undispatched row never claimed by new code has no `attempt`.
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (10).").await;
    let reference = card.authored_skill.clone().unwrap();
    let attempt = review::publication_attempt(&f.state, &f.owner, &reference)
        .await
        .unwrap();
    assert_eq!(attempt, 0);
    review::record_pre_claim_failure(
        &f.state,
        &f.owner,
        &reference,
        &card,
        attempt,
        review::PublicationFailureCode::NyxidRefused,
    )
    .await
    .unwrap();
    assert_eq!(
        stored(&f, &row.id).await.failure_code.as_deref(),
        Some("nyxid_refused")
    );
    // A changed base is recorded on it as well.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.sha256":"f".repeat(64)}},
        )
        .await
        .unwrap();
    assert!(
        review::approval_binding(
            &f.state,
            &f.owner,
            &row.agent_id,
            &row.id,
            row.revision,
            reference.skills_revision,
            &crate::services::assistant_agent_learning_review::UnavailableReader
        )
        .await
        .is_err()
    );
    assert_eq!(
        stored(&f, &row.id).await.failure_code.as_deref(),
        Some("base_changed")
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.chat.agent_id},
            doc! {"$set":{"skills.0.sha256":&base.sha}},
        )
        .await
        .unwrap();

    // The single operator release works on a dispatched pre-#1828 row.
    let (dispatched, dispatched_card) = legacy_draft(&f, &base, "Search the tracker (11).").await;
    review::reject(
        &f.state,
        &f.owner,
        &row.agent_id,
        &row.id,
        row.revision,
        "rejected",
    )
    .await
    .unwrap();
    let dispatched = dispatch_and_deploy(&f, &dispatched, &dispatched_card).await;
    let operator = crate::services::audit_service::AuditActor {
        user_id: "operator".into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    review::operator_release_target(
        &f.state,
        &operator,
        &dispatched.id,
        &dispatched.publication.as_ref().unwrap().operation_id,
        "INC-1812",
    )
    .await
    .unwrap();
    let released = stored(&f, &dispatched.id).await;
    assert_eq!(released.failure_code.as_deref(), Some("operator_released"));
    assert!(barrier(&f, &base).await.is_none());
    let used = acknowledgement(&f, &dispatched_card.id).await;
    assert_eq!(
        card_preview(&f, &released, &used).await["actions"],
        json!(["reprepare", "discard"])
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_late_landing_of_the_old_request_makes_the_rebuild_a_version_conflict() {
    let f = fixture("recovery_late_landing").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (12).").await;
    let old_bytes = legacy_bytes(&f, &row).await;
    let old = row.publication.clone().unwrap();
    reprepare(&f, &row, Some(&card)).await.unwrap();
    let rebuilt = stored(&f, &row.id).await;
    let new_card = rebuilt_card(&f, &rebuilt).await;
    server.reset().await;
    mount_base(&f, &server, &base).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/skill-format/validate"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":{"valid":true,"violations":[]}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/skills/{}", base.skill_id)))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(json!({"code":"SKILL_VERSION_EXISTS"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    // The stalled pre-#1828 request landed first, with the old package.
    mount_version(&server, &f.owner, &base.skill_id, &old, old_bytes).await;
    assert!(finish(&f, new_card.clone(), true).await.is_err());
    server.verify().await;
    let after = stored(&f, &row.id).await;
    let p = after.publication.clone().unwrap();
    assert_eq!(after.status, "publication_failed");
    assert_eq!(after.failure_code.as_deref(), Some("version_conflict"));
    assert!(p.started && p.uncertain_dispatch && p.verified_at.is_none());
    assert!(p.dispatched_at.is_some());
    let held = barrier(&f, &base).await.unwrap();
    assert_eq!(
        (held.operation_id.as_str(), held.state.as_str()),
        (p.operation_id.as_str(), "uncertain")
    );
    let used = acknowledgement(&f, &new_card.id).await;
    let preview = card_preview(&f, &after, &used).await;
    assert_eq!(preview["failure_code"], "version_conflict");
    for action in ["publish", "retry", "reprepare", "discard"] {
        assert!(
            !preview["actions"]
                .as_array()
                .unwrap()
                .contains(&json!(action))
        );
    }
    let agent =
        crate::services::assistant_team_service::agent(&f.state.db, &f.owner, &row.agent_id)
            .await
            .unwrap();
    assert!(agent.skills.iter().all(|pin| pin.version == "1.0"));
    f.state.db.drop().await.unwrap();
}

/// What a pre-#1828 server left after an approved attempt failed before
/// dispatch: `published_unpinned` with its lease cleared, or `publishing`
/// after a crash with the lease expired. The card was used.
async fn pre_dispatch_failure(
    f: &Fixture,
    row: &AssistantAgentLearningProposal,
    card: &AssistantAcknowledgement,
    status: &str,
) -> (AssistantAgentLearningProposal, AssistantAcknowledgement) {
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_one(doc! {"_id":&card.id}, doc! {"$set":{"status":"used"}})
        .await
        .unwrap();
    let lease = if status == "publishing" {
        bson::Bson::DateTime(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::hours(30),
        ))
    } else {
        bson::Bson::Null
    };
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":status,
            "failure_code":"publication_retry_required","publication.started":false,
            "publication.approved_by":&f.owner,"publication.acknowledgement_id":&card.id,
            "publication.approval_digest":card.arguments_digest.as_ref().unwrap(),
            "publication.lease_id":"pre-1828-lease","publication.lease_expires_at":lease}},
        )
        .await
        .unwrap();
    assert_pre_1828_shape(f, &row.id).await;
    (stored(f, &row.id).await, acknowledgement(f, &card.id).await)
}

async fn pre_dispatch_failure_rebuilds_and_publishes(name: &str, status: &str) {
    let f = fixture(name).await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (13).").await;
    let (row, used) = pre_dispatch_failure(&f, &row, &card, status).await;
    let shown_status = if status == "publishing" {
        "publishing"
    } else {
        "publication_failed"
    };
    let preview = card_preview(&f, &row, &used).await;
    assert_eq!(preview["actions"], json!(["reprepare", "discard"]));
    assert_eq!(preview["failure_code"], LEGACY);
    assert_eq!(preview["status"], shown_status);
    let listed = review::list(&f.state, &f.owner, &row.agent_id, true)
        .await
        .unwrap();
    assert_eq!(listed[0].failure_code.as_deref(), Some(LEGACY));
    assert_eq!(listed[0].status, shown_status);
    assert!(matches!(finish(&f, used.clone(), true).await,
        Err(AppError::Conflict(message)) if message == WITHHELD));
    assert_eq!(writes(&server).await, 0);
    reprepare(&f, &row, Some(&used)).await.unwrap();
    let rebuilt = stored(&f, &row.id).await;
    assert_eq!(rebuilt.status, "pending");
    assert_eq!(rebuilt.publication.as_ref().unwrap().package_format, 2);
    assert_tools_preserved(&publish_rebuilt(&f, &server, &base, &row).await);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_pre_dispatch_failure_left_published_unpinned_rebuilds_and_publishes() {
    pre_dispatch_failure_rebuilds_and_publishes(
        "recovery_pre_dispatch_unpinned",
        "published_unpinned",
    )
    .await;
}

#[tokio::test]
async fn recovery_pre_dispatch_crash_left_publishing_rebuilds_and_publishes() {
    pre_dispatch_failure_rebuilds_and_publishes("recovery_pre_dispatch_publishing", "publishing")
        .await;
}

#[tokio::test]
async fn recovery_pre_dispatch_failures_can_be_discarded() {
    let f = fixture("recovery_pre_dispatch_discard").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    for (step, status) in [(14, "published_unpinned"), (15, "publishing")] {
        let (row, card) = legacy_draft(&f, &base, &format!("Search the tracker ({step}).")).await;
        let (row, used) = pre_dispatch_failure(&f, &row, &card, status).await;
        assert!(
            card_preview(&f, &row, &used).await["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("discard"))
        );
        review::reject(
            &f.state,
            &f.owner,
            &row.agent_id,
            &row.id,
            row.revision,
            "rejected",
        )
        .await
        .unwrap();
        assert_eq!(stored(&f, &row.id).await.status, "rejected");
    }
    // A dispatched attempt still refuses to be discarded.
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (16).").await;
    let (row, _) = pre_dispatch_failure(&f, &row, &card, "publishing").await;
    f.state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.started":true}},
        )
        .await
        .unwrap();
    assert!(matches!(
        review::reject(
            &f.state,
            &f.owner,
            &row.agent_id,
            &row.id,
            row.revision,
            "rejected"
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(stored(&f, &row.id).await.status, "publishing");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_card_left_unposted_after_a_committed_rebuild_is_reported_and_raised_later() {
    let f = fixture("recovery_rebuild_card_pending").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (17).").await;
    server.reset().await;
    mount_base_after(&f, &server, &base, std::time::Duration::from_millis(700)).await;
    let keys = f
        .state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
    // The chat key stops being usable while NyxID reads the base (its card
    // authority was already checked): the swap commits, the card cannot be
    // raised.
    let (result, ()) = tokio::join!(reprepare(&f, &row, Some(&card)), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        keys.update_one(
            doc! {"_id":&card.api_key_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    });
    let result = result.unwrap();
    assert_eq!(result["status"], "reprepared");
    assert_eq!(result["card_pending"], true);
    let rebuilt = stored(&f, &row.id).await;
    let p = rebuilt.publication.clone().unwrap();
    assert!(p.package_format == 2 && p.review_card_pending);
    assert_eq!(
        audits(&f, "assistant_learning_proposal_reprepared", &row.id)
            .await
            .len(),
        1
    );
    keys.update_one(
        doc! {"_id":&card.api_key_id},
        doc! {"$set":{"is_active":true}},
    )
    .await
    .unwrap();
    assert_eq!(
        card_preview(&f, &rebuilt, &card).await["actions"],
        json!(["show_updated_draft"])
    );
    let raised = reprepare(&f, &row, Some(&card)).await.unwrap();
    assert!(raised.get("card_pending").is_none());
    let new_card = rebuilt_card(&f, &row).await;
    assert_eq!(raised["acknowledgement_id"], new_card.id);
    assert!(
        !stored(&f, &row.id)
            .await
            .publication
            .unwrap()
            .review_card_pending
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_stale_release_frees_only_the_reservation_of_a_draft_invalidated_after_dispatch() {
    let f = fixture("recovery_release_invalidated").await;
    let server = setup_ornn(&f).await;
    let base = tool_base(&f, &server).await;
    let (row, card) = legacy_draft(&f, &base, "Search the tracker (18).").await;
    // Pre-#1828: dispatched, then invalidated (body removed) before deploy.
    let proposals = f
        .state
        .db
        .collection::<bson::Document>(PROPOSALS_COLLECTION_NAME);
    proposals.update_one(doc! {"_id":&row.id}, doc! {"$set":{"status":"invalidated",
            "failure_code":"evidence_unavailable","body_bytes":0_i64,"publication.started":true,
            "publication.approved_by":&f.owner,"publication.acknowledgement_id":&card.id,
            "publication.approval_digest":card.arguments_digest.as_ref().unwrap(),
            "publication.lease_id":"pre-1828-lease",
            "publication.lease_expires_at":bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(30))},
            "$unset":{"body_encrypted":""}})
        .await.unwrap();
    assert_pre_1828_shape(&f, &row.id).await;
    let migrations = f
        .state
        .db
        .collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME);
    migrations.delete_many(doc! {}).await.unwrap();
    // Without a draft the migration cannot classify it until an operator
    // records the target; the next start then reserves it.
    assert_eq!(
        review::migrate_publication_targets(&f.state, false)
            .await
            .unwrap(),
        review::MigrationOutcome::Unresolved
    );
    proposals
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"publication.target_kind":"update",
            "publication.target_skill_id":&base.skill_id}},
        )
        .await
        .unwrap();
    assert_eq!(
        review::migrate_publication_targets(&f.state, false)
            .await
            .unwrap(),
        review::MigrationOutcome::Complete
    );
    assert_eq!(barrier(&f, &base).await.unwrap().state, "uncertain");
    age(&f, &row, 25).await;
    // A new draft of the same skill version is blocked by that reservation.
    let mut revision = input(&f.chat.agent_id);
    revision["base_skill"] = json!({"skill_id":base.skill_id,"version":"1.0"});
    let (next, next_card) = make_with_access(&f, revision, true).await;
    let blocked = card_preview(&f, &next, &next_card).await;
    assert_eq!(blocked["failure_code"], "target_busy");
    assert_eq!(blocked["actions"], json!(["deny"]));

    let operator = admin(&f).await;
    for (exact, versions, expected) in [
        (
            ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":base.skill_id}})),
            json!([{"version":"1.0"}]),
            "skip:landed",
        ),
        // A list that contains the version contradicts the 404.
        (
            version_not_found(),
            json!([{"version":"1.1"},{"version":"1.0"}]),
            "skip:uncertain",
        ),
        (
            version_not_found(),
            json!([{"version":"1.0"}]),
            "release_reservation",
        ),
    ] {
        server.reset().await;
        mount_exact(&server, &base, exact).await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/skills/{}/versions", base.skill_id)))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data":{"items":versions}})),
            )
            .mount(&server)
            .await;
        let dry = release_stale(&f, operator.clone(), json!({}))
            .await
            .unwrap();
        assert_eq!(decisions(&dry), [expected]);
        assert!(barrier(&f, &base).await.is_some());
    }
    let applied = release_stale(&f, operator, json!({"dry_run":false}))
        .await
        .unwrap();
    assert_eq!(decisions(&applied), ["released_reservation_audited"]);
    let released = stored(&f, &row.id).await;
    assert_eq!(released.status, "invalidated");
    assert_eq!(
        released.failure_code.as_deref(),
        Some("evidence_unavailable")
    );
    let p = released.publication.unwrap();
    assert!(!p.started && !p.uncertain_dispatch);
    assert_eq!(p.release_evidence.unwrap().latest_version, "1.0");
    assert!(barrier(&f, &base).await.is_none());
    let audit = audits(
        &f,
        "assistant_learning_publication_target_released",
        &row.id,
    )
    .await;
    assert_eq!(audit.len(), 1);
    let data = audit[0].get_document("event_data").unwrap();
    assert!(data.get_bool("reservation_only").unwrap());
    assert_eq!(data.get_str("proposal_status").unwrap(), "invalidated");
    assert_eq!(data.get_str("evidence").unwrap(), "absence_only");
    // The new draft can publish now.
    let freed = card_preview(&f, &next, &next_card).await;
    assert_eq!(freed["actions"], json!(["deny", "publish"]));
    assert_eq!(writes(&server).await, 0);
    assert_owner_identity(&f, &server).await;
    f.state.db.drop().await.unwrap();
}
