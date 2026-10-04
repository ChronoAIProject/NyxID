//! Owner-authorized updates and durable reconnect watches. No credentials enter
//! the setup guidance, tool result, card, audit or update request.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        machine_update::{COLLECTION_NAME, MachineUpdate},
        node::Node,
    },
    mw::auth::AuthUser,
    services::{
        assistant_acknowledgement_service as acks, assistant_links::AssistantPage,
        machine_service as machines, machine_update_service as updates,
    },
};
use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use nyxid_machine::Operation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

static UPDATER: std::sync::LazyLock<nyxid_machine_updater::release_image::VerifiedUpdater> =
    std::sync::LazyLock::new(Default::default);

#[cfg(test)]
tokio::task_local! { static TEST_UPDATER_IMAGE: String; }

async fn verified_updater_image() -> Option<String> {
    #[cfg(test)]
    if let Ok(image) = TEST_UPDATER_IMAGE.try_with(Clone::clone) {
        return Some(image);
    }
    UPDATER.image(updates::TARGET).await
}

pub async fn updater_image(auth: AuthUser) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    // Return a useful pending state promptly; verification continues once in
    // the shared cache, rather than restarting after a browser request timeout.
    let resolution = tokio::spawn(verified_updater_image());
    let image = tokio::time::timeout(std::time::Duration::from_millis(75), resolution)
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
    Ok(Json(json!({"version": updates::TARGET, "image": image,
        "status": if image.is_some() { "verified" } else { "verifying" }})))
}

#[derive(Serialize)]
pub struct Status {
    node_id: String,
    current_version: String,
    target_version: &'static str,
    update_available: bool,
    updater_ready: bool,
    updater: Option<nyxid_machine::update::CompanionStatus>,
    updater_guidance: Option<&'static str>,
    installation: nyxid_machine::update::Installation,
    automatic: bool,
    phase: String,
    code: Option<String>,
    guidance: Option<&'static str>,
    settings_path: String,
}
impl Status {
    fn new(node: &Node, row: MachineUpdate) -> Self {
        let updater = companion_status(node);
        let updater_guidance = updater
            .as_ref()
            .and_then(|s| s.code.as_deref())
            .and_then(nyxid_machine::update::failure_guidance);
        Self {
            updater,
            updater_guidance,
            node_id: node.id.clone(),
            current_version: updates::current(node).into(),
            target_version: updates::TARGET,
            update_available: updates::update_available(node),
            updater_ready: node.machine.as_ref().is_some_and(|m| m.updater_ready),
            installation: updates::installation(node),
            automatic: row.automatic,
            phase: row.phase,
            guidance: row
                .code
                .as_deref()
                .and_then(nyxid_machine::update::failure_guidance),
            code: row.code,
            settings_path: AssistantPage::MachineSettings { node: &node.id }.path(),
        }
    }
}

fn companion_status(node: &Node) -> Option<nyxid_machine::update::CompanionStatus> {
    updates::companion_status(node)
}

pub async fn list(State(state): State<AppState>, auth: AuthUser) -> AppResult<Json<Vec<Status>>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let nodes: Vec<Node> =
        crate::services::node_service::list_user_nodes(&state.db, &auth.user_id.to_string())
            .await?
            .into_iter()
            .map(|row| row.node)
            .collect();
    let ids: Vec<_> = nodes
        .iter()
        .filter(|n| n.machine.as_ref().is_some_and(|m| m.enabled()))
        .map(|n| n.id.as_str())
        .collect();
    let rows: Vec<MachineUpdate> = state
        .db
        .collection(COLLECTION_NAME)
        .find(doc! {"_id":{"$in":&ids}})
        .await?
        .try_collect()
        .await?;
    let mut rows: std::collections::HashMap<_, _> =
        rows.into_iter().map(|r| (r.node_id.clone(), r)).collect();
    Ok(Json(
        nodes
            .iter()
            .filter(|n| ids.contains(&n.id.as_str()))
            .map(|n| {
                Status::new(
                    n,
                    rows.remove(&n.id)
                        .filter(|r| r.user_id == n.user_id)
                        .unwrap_or_else(|| MachineUpdate::new(&n.id, &n.user_id)),
                )
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    automatic: bool,
}
pub async fn policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<Policy>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let node = updates::owner_node(&state.db, &auth.user_id.to_string(), &id).await?;
    updates::policy(&state.db, &node, input.automatic).await?;
    Ok(Json(json!({"automatic":input.automatic})))
}
pub async fn start(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let node = updates::owner_node(&state.db, &actor, &id).await?;
    Ok(Json(begin(&state, &node, &actor, None, false).await?))
}

pub async fn begin(
    state: &AppState,
    node: &Node,
    actor: &str,
    conversation: Option<&str>,
    automatic: bool,
) -> AppResult<Value> {
    let legacy = updates::legacy_companion(node);
    let manual = legacy || !node.machine.as_ref().is_some_and(|m| m.updater_ready);
    let row = updates::begin(&state.db, node, actor, conversation, automatic, manual).await?;
    let path = AssistantPage::MachineSettings { node: &node.id }.path();
    if legacy {
        return Ok(json!({
            "status": "manual_step",
            "updater": companion_status(node),
            "machine": node.id,
            "target_version": updates::TARGET,
            "url": AssistantPage::MachineSettings { node: &node.id }.url(&state.config.frontend_url),
            "steps": [
                "Open a terminal on the computer running Docker.",
                "Open the machine link and paste its pinned Replace updater command. It keeps the machine container and update volume.",
                "Wait for the companion to report its version; this thread resumes automatically."
            ],
            "note": "Updater predates self-update. Replace it once. End your turn now; NyxID watches companion metadata and expiry. If the owner identifies another granted native machine on the same Docker host, offer to run the replacement there after Docker inspection and an owner card. Never guess a host or run inside the target container."
        }));
    }
    if manual {
        return Ok(
            json!({"status":"manual_step","updater":companion_status(node),"machine":node.id,"target_version":updates::TARGET,
            "url":AssistantPage::MachineSettings{node:&node.id}.url(&state.config.frontend_url),
            "steps":["Open a terminal on the computer running Docker (or the native machine).","Open the machine link and paste its prefilled update command into that terminal.","Wait for the machine to reconnect; this thread resumes automatically."],
            "note":"End your turn now. NyxID watches reconnection and expiry. If the owner identifies a different granted machine on the Docker host, offer to run the migration there after Docker inspection and an owner card. Never guess a host or run inside the container being updated."}),
        );
    }
    let result = super::machine_tools::dispatch(
        state,
        node,
        Operation::Upgrade,
        json!({"version":updates::TARGET,"automatic":automatic,"owner_rollback":false}),
    )
    .await;
    if !result.as_ref().is_ok_and(|r| r["accepted"] == true) {
        updates::finish(&state.db, &row, "failed", Some("update_not_accepted")).await?;
        return Ok(
            json!({"status":"failed","reason":"Machine did not accept the update; it may be busy or its updater unavailable.","settings_path":path}),
        );
    }
    Ok(
        json!({"status":"queued","updater":companion_status(node),"machine":node.id,"target_version":updates::TARGET,"settings_path":path,"note":"End your turn now. This thread wakes on reconnect or failure. Then verify the version, AX computer state and browser snapshot before continuing."}),
    )
}

pub async fn tool(
    state: &AppState,
    chat: &acks::ChatAuthority,
    args: &Value,
) -> AppResult<(Value, bool)> {
    machines::caller(chat)?;
    if chat.turn_stopped {
        return Err(AppError::MachineTurnStopped);
    }
    let selector = args["machine"].as_str().unwrap_or_default();
    let nodes = machines::visible_nodes(&state.db, chat).await?;
    let mut found = nodes
        .iter()
        .filter(|n| n.id == selector || n.name == selector);
    let node = found
        .next()
        .ok_or_else(|| AppError::NodeNotFound("Machine not found".into()))?;
    if found.next().is_some() {
        return Err(AppError::ValidationError(
            "Use the machine ID; several names match".into(),
        ));
    }
    if !machines::granted(chat, node) {
        return Ok((
            super::machine_tools::permission(state, chat, "machine", &node.id, &node.name).await?,
            true,
        ));
    }
    updates::owner_node(&state.db, &chat.user_id, &node.id).await?;
    let host = if let Some(selector) = args["host_machine"].as_str() {
        let mut hosts = nodes
            .iter()
            .filter(|n| n.id == selector || n.name == selector);
        let host = hosts
            .next()
            .ok_or_else(|| AppError::NodeNotFound("Docker host machine not found".into()))?;
        if hosts.next().is_some() || host.id == node.id {
            return Err(AppError::ValidationError(
                "Choose a different, unambiguous machine on the Docker host".into(),
            ));
        }
        if !machines::granted(chat, host) {
            return Ok((
                super::machine_tools::permission(state, chat, "machine", &host.id, &host.name)
                    .await?,
                true,
            ));
        }
        Box::pin(crate::services::machine_access_service::authorize(
            &state.db,
            chat,
            host,
            Operation::Exec,
            &json!({}),
        ))
        .await?;
        if !host.machine.as_ref().is_some_and(|m| m.shell)
            || updates::installation(host) == nyxid_machine::update::Installation::Container
        {
            return Err(AppError::ValidationError(
                "Use a native machine on the Docker host with shell access".into(),
            ));
        }
        if updates::legacy_companion(node)
            && !nyxid_machine::update::version(updates::current(host))
                .is_ok_and(|v| v >= nyxid_machine::update::version("0.41.4").expect("release"))
        {
            return Err(AppError::ValidationError(
                "Update the Docker host machine node to 0.41.4 or newer before agent-run companion replacement, or use the guided host command".into(),
            ));
        }
        updates::owner_node(&state.db, &chat.user_id, &host.id).await?;
        Some(host)
    } else {
        None
    };
    let legacy = updates::legacy_companion(node);
    let mut canonical = json!({"machine":node.id,"target_version":updates::TARGET});
    if legacy {
        canonical["replace_companion"] = json!(true);
    }
    if let Some(host) = host {
        let Some(image) = verified_updater_image().await else {
            return Ok((
                json!({"status":"verifying", "reason":"Verifying the updater image, try again shortly."}),
                true,
            ));
        };
        canonical["updater_image"] = json!(image);
        let container = args["container"]
            .as_str()
            .filter(|s| nyxid_machine::update::container_name(s))
            .ok_or_else(|| {
                AppError::ValidationError(
                    "Provide the owner-identified Docker container name".into(),
                )
            })?;
        let inspect = super::machine_tools::dispatch(
            state,
            host,
            Operation::ContainerInspect,
            json!({"container":container,"replace_companion":legacy,"conversation_id":chat.conversation_id,"turn_id":chat.turn_id.clone().unwrap_or_default()}),
        )
        .await?;
        if inspect["docker_access"] != true {
            return Err(AppError::ValidationError(
                "Docker access or target container verification failed on that host".into(),
            ));
        }
        canonical["host_machine"] = json!(host.id);
        canonical["container"] = json!(container);
        canonical["container_id"] = inspect["container_id"].clone();
        if legacy {
            let companion_id = inspect["companion_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    AppError::ValidationError(
                        "Updater container verification failed on that host".into(),
                    )
                })?;
            canonical["companion_id"] = json!(companion_id);
        }
    }
    if let Some(id) = args["acknowledgement_id"].as_str()
        && acks::consume_action(&state.db, chat, id, "nyxid__machine_update", &canonical).await?
    {
        if let Some(host) = host {
            let row = Box::pin(updates::begin(
                &state.db,
                node,
                &chat.user_id,
                Some(&chat.conversation_id),
                false,
                true,
            ))
            .await?;
            canonical["version"] = json!(updates::TARGET);
            canonical["conversation_id"] = json!(chat.conversation_id);
            canonical["turn_id"] = json!(chat.turn_id.clone().unwrap_or_default());
            let result =
                super::machine_tools::dispatch(state, host, Operation::ContainerMigrate, canonical)
                    .await?;
            if result["accepted"] != true {
                updates::finish(&state.db, &row, "failed", Some("host_migration_failed")).await?;
            }
            return Ok((
                json!({"status":if result["accepted"]==true {"migrating"}else{"failed"},"updater":companion_status(node),"note":if legacy {"End your turn; companion version metadata or expiry resumes this thread. The machine and its volumes are retained."} else {"End your turn; the target machine reconnect or expiry watch resumes this thread. Verify version, AX and browser health before continuing."}}),
                false,
            ));
        }
        return Ok((
            Box::pin(begin(
                state,
                node,
                &chat.user_id,
                Some(&chat.conversation_id),
                false,
            ))
            .await?,
            false,
        ));
    }
    let summary = if legacy {
        format!(
            "Replace the legacy updater for machine {} with the attested updater for {}. {} Keep the machine container and update volume. Docker socket access gives the updater host-root authority.",
            node.name,
            updates::TARGET,
            host.map(|h| format!(
                "Run on owner-identified host machine {} after its successful Docker inspection.",
                h.name
            ))
            .unwrap_or_else(|| "Guide the one-time pinned host command.".into())
        )
    } else {
        format!("Update machine {} from {} to {}. {} This restarts the target and interrupts its work. Docker socket access gives the updater host-root authority; it installs only attested official images.",node.name,updates::current(node),updates::TARGET,host.map(|h|format!("Run the migration on owner-identified host machine {} after its successful Docker inspection.",h.name)).unwrap_or_else(||"Guide the one-time host command if no updater is installed.".into()))
    };
    // The owner-card transaction is large in debug builds and this tool also
    // runs beneath universal MCP dispatch on the default thread stack.
    let card = Box::pin(acks::request(
        &state.db,
        chat,
        acks::Request {
            kind: "action",
            service: Some((&node.id, &node.id, &node.name)),
            tool: Some("nyxid__machine_update"),
            arguments: Some(&canonical),
            summary: &summary,
            platform: false,
        },
    ))
    .await?;
    let mut result = acks::refusal(&card);
    let previous = updates::get(&state.db, node).await?;
    result["previous_update"] = serde_json::to_value(Status::new(node, previous))
        .map_err(|_| AppError::Internal("Could not encode update status".into()))?;
    if legacy {
        result["guidance"] = json!(
            "Updater predates self-update. Replace it once. After owner approval, guide the pinned host command in Machines, or use an owner-identified different granted native machine on the Docker host. End your turn and wait for companion metadata or expiry."
        );
    }
    Ok((result, true))
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            timer.tick().await;
            if sweep(&state).await.is_err() {
                tracing::warn!("Machine update reconciliation deferred");
            }
        }
    });
}
async fn sweep(state: &AppState) -> AppResult<()> {
    let mut rows=state.db.collection::<MachineUpdate>(COLLECTION_NAME).find(doc!{"$or":[{"automatic":true},{"notify_pending":true},{"phase":{"$in":["manual_step","queued","verifying","downloading","restarting"]}}]}).await?;
    while let Some(row) = rows.try_next().await? {
        let Some(node) = updates::node_for_record(&state.db, &row).await? else {
            updates::finish(&state.db, &row, "failed", Some("machine_unavailable")).await?;
            state
                .db
                .collection::<MachineUpdate>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &row.node_id, "user_id": &row.user_id},
                    doc! {"$set": {"automatic": false, "notify_pending": false}},
                )
                .await?;
            continue;
        };
        if row.pending() {
            let report = if node.status == crate::models::node::NodeStatus::Online
                && node.machine.as_ref().is_some_and(|m| m.updater_ready)
            {
                super::machine_tools::dispatch(state, &node, Operation::UpgradeStatus, json!({}))
                    .await
                    .ok()
                    .and_then(|v| serde_json::from_value(v["progress"].clone()).ok())
            } else {
                None
            };
            updates::observe(&state.db, &node, report.as_ref()).await?;
        } else if row.automatic
            && updates::update_available(&node)
            && node.status == crate::models::node::NodeStatus::Online
            && node.machine.as_ref().is_some_and(|m| m.updater_ready)
            && updates::idle(&state.db, &node.id).await?
            && row.updated_at < chrono::Utc::now() - chrono::Duration::minutes(15)
        {
            let _ = begin(state, &node, &row.user_id, None, true).await;
        }
        if row.notify_pending {
            // The persisted notification claim is cross-replica; no duplicate fanout.
            let claim=state.db.collection::<MachineUpdate>(COLLECTION_NAME).update_one(doc!{"_id":&row.node_id,"updated_at":bson::DateTime::from_chrono(row.updated_at),"notify_pending":true},doc!{"$set":{"notify_pending":false}}).await?;
            if claim.modified_count == 1 {
                notify(state, &node, &row).await;
            }
        }
    }
    Ok(())
}
async fn notify(state: &AppState, node: &Node, row: &MachineUpdate) {
    let mut recipients = vec![
        row.requested_by
            .as_deref()
            .unwrap_or(&row.user_id)
            .to_owned(),
    ];
    if let Ok(members) =
        crate::services::org_service::list_members_for_org(&state.db, &node.user_id, false).await
    {
        recipients.extend(
            members
                .into_iter()
                .filter(|m| m.role == crate::models::org_membership::OrgRole::Admin)
                .map(|m| m.member_user_id),
        );
    }
    recipients.sort();
    recipients.dedup();
    let event = json!({
        "event_id": row.attempt_id,
        "payload": format!("{}: {} (target {}). {}",node.name,row.phase,
            row.target_version.as_deref().unwrap_or(updates::TARGET),
            AssistantPage::MachineSettings {node:&node.id}.url(&state.config.frontend_url)),
    });
    for recipient in recipients {
        let _ = crate::services::notification_service::send_trigger_notification(
            &state.db,
            &state.config,
            &state.http_client,
            state.fcm_auth.as_deref(),
            state.apns_auth.as_deref(),
            &recipient,
            "Machine update",
            &event,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        nyxbot_channel::{NyxbotWatch, WATCHES_COLLECTION_NAME as WATCHES},
    };
    use crate::services::{
        assistant_authority_tests::{fixture, orchestrator_fixture},
        machine_integration_tests::{node, peer},
    };

    #[tokio::test]
    async fn legacy_detection_requires_reporting_node_and_live_container_companion() {
        let f = orchestrator_fixture("legacy_detection").await;
        let mut node = node(&f, &f.owner).await;
        node.machine.as_mut().unwrap().installation =
            Some(nyxid_machine::update::Installation::Container);
        node.machine.as_mut().unwrap().updater_ready = true;
        for (version, legacy) in [
            ("0.41.0", false),
            ("0.41.3", false),
            ("unknown", false),
            ("0.41.4", true),
            ("0.42.0", true),
        ] {
            node.metadata = Some(crate::models::node::NodeMetadata {
                agent_version: Some(version.into()),
                os: None,
                arch: None,
                ip_address: None,
                provisioning_source: None,
            });
            assert_eq!(updates::legacy_companion(&node), legacy, "{version}");
        }
        node.machine.as_mut().unwrap().updater_ready = false;
        assert!(companion_status(&node).is_none());
        node.machine.as_mut().unwrap().updater_ready = true;
        node.machine.as_mut().unwrap().updater = Some(nyxid_machine::update::CompanionStatus {
            version: "0.41.4".into(),
            phase: "current".into(),
            ..Default::default()
        });
        assert!(!updates::legacy_companion(&node));
        assert_eq!(companion_status(&node).unwrap().version, "0.41.4");
        node.machine
            .as_mut()
            .unwrap()
            .updater
            .as_mut()
            .unwrap()
            .code = Some("private diagnostic".into());
        let response = serde_json::to_value(Status::new(
            &node,
            MachineUpdate::new(&node.id, &node.user_id),
        ))
        .unwrap();
        assert_eq!(response["updater"]["phase"], "legacy");
        assert!(
            response["updater_guidance"]
                .as_str()
                .unwrap()
                .contains("Replace it once")
        );
        assert!(!response.to_string().contains("private diagnostic"));
        node.machine.as_mut().unwrap().installation =
            Some(nyxid_machine::update::Installation::Native);
        assert!(companion_status(&node).is_none());
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_guided_specialist_card_wakes_on_metadata_without_machine_restart() {
        let mut f = fixture("legacy_guided").await;
        let mut node = node(&f, &f.owner).await;
        node.machine.as_mut().unwrap().installation =
            Some(nyxid_machine::update::Installation::Container);
        node.machine.as_mut().unwrap().updater_ready = true;
        node.machine.as_mut().unwrap().updater =
            Some(nyxid_machine::update::CompanionStatus::legacy());
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .replace_one(doc! {"_id": &node.id}, &node)
            .await
            .unwrap();
        let (denied, _) = tool(&f.state, &f.chat, &json!({"machine": node.id}))
            .await
            .unwrap();
        let denied = f
            .state
            .db
            .collection::<AssistantAcknowledgement>(ACKS)
            .find_one(doc! {"_id": denied["acknowledgement_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(denied.kind, "machine");
        f.chat.machine_node_ids.push(node.id.clone());
        let (card, _) = tool(&f.state, &f.chat, &json!({"machine": node.id}))
            .await
            .unwrap();
        assert_eq!(card["previous_update"]["updater"]["phase"], "legacy");
        assert!(
            card["guidance"]
                .as_str()
                .unwrap()
                .contains("different granted native machine")
        );
        let id = card["acknowledgement_id"].as_str().unwrap();
        acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        let (result, error) = tool(
            &f.state,
            &f.chat,
            &json!({"machine": node.id, "acknowledgement_id": id}),
        )
        .await
        .unwrap();
        assert!(!error);
        assert_eq!(result["status"], "manual_step");
        assert_eq!(result["updater"]["phase"], "legacy");
        assert!(
            result["url"]
                .as_str()
                .unwrap()
                .contains("/assistant/machines")
        );
        assert!(result.to_string().contains("End your turn"));
        let row = updates::get(&f.state.db, &node).await.unwrap();
        assert!(row.replace_companion);
        // Even a fresh machine-connected progress report cannot complete this goal.
        updates::observe(
            &f.state.db,
            &node,
            Some(&nyxid_machine::update::Progress {
                target: updates::TARGET.into(),
                phase: nyxid_machine::update::Phase::Connected,
                started_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                code: None,
                updater: None,
            }),
        )
        .await
        .unwrap();
        assert!(updates::get(&f.state.db, &node).await.unwrap().pending());
        let watch = f
            .state
            .db
            .collection::<NyxbotWatch>(WATCHES)
            .find_one(doc! {"connect_link_id": &row.attempt_id})
            .await
            .unwrap()
            .unwrap();
        let original_connection = node.connected_at;
        node.machine.as_mut().unwrap().updater = Some(nyxid_machine::update::CompanionStatus {
            version: "0.41.4".into(),
            phase: "current".into(),
            ..Default::default()
        });
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .replace_one(doc! {"_id": &node.id}, &node)
            .await
            .unwrap();
        super::super::nyxbot::machine_update_watch(&f.state, &watch)
            .await
            .unwrap();
        assert_eq!(node.connected_at, original_connection);
        assert_eq!(
            updates::get(&f.state.db, &node).await.unwrap().phase,
            "connected"
        );
        let conversation =
            crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap();
        let transcript = serde_json::to_string(&conversation).unwrap();
        assert!(transcript.contains("legacy updater replacement completed"));
        assert!(transcript.contains("machine_update_finished"));
        // Retry/expiry remains durable and offers fixed, actionable guidance.
        node.machine.as_mut().unwrap().updater =
            Some(nyxid_machine::update::CompanionStatus::legacy());
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .replace_one(doc! {"_id": &node.id}, &node)
            .await
            .unwrap();
        begin(&f.state, &node, &f.owner, Some(&f.row.id), false)
            .await
            .unwrap();
        f.state.db.collection::<bson::Document>(COLLECTION_NAME)
            .update_one(doc! {"_id": &node.id}, doc! {"$set": {"deadline": bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::seconds(1))}}).await.unwrap();
        updates::observe(&f.state.db, &node, None).await.unwrap();
        assert_eq!(
            updates::get(&f.state.db, &node)
                .await
                .unwrap()
                .code
                .as_deref(),
            Some("update_companion:companion_replacement_timeout")
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn companion_outcomes_reach_status_and_nyxbot_without_hiding_machine_success() {
        let f = orchestrator_fixture("companion_update_status").await;
        let mut node = node(&f, &f.owner).await;
        node.machine.as_mut().unwrap().installation =
            Some(nyxid_machine::update::Installation::Container);
        node.machine.as_mut().unwrap().updater = Some(nyxid_machine::update::CompanionStatus {
            version: "0.41.0".into(),
            digest: Some(format!("sha256:{}", "a".repeat(64))),
            target_version: Some(updates::TARGET.into()),
            phase: "failed".into(),
            code: Some("update_companion:attestation_invalid".into()),
        });
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .replace_one(doc! {"_id": &node.id}, &node)
            .await
            .unwrap();
        let mut row = MachineUpdate::new(&node.id, &node.user_id);
        row.phase = "connected".into();
        let response = serde_json::to_value(Status::new(&node, row)).unwrap();
        assert_eq!(response["phase"], "connected");
        assert_eq!(response["updater"]["version"], "0.41.0");
        assert_eq!(
            response["updater"]["code"],
            "update_companion:attestation_invalid"
        );
        assert!(
            response["updater_guidance"]
                .as_str()
                .unwrap()
                .contains("do not bypass")
        );
        let (result, refused) = tool(&f.state, &f.chat, &json!({"machine": node.id}))
            .await
            .unwrap();
        assert!(refused, "Updates still require an owner card");
        assert_eq!(result["previous_update"]["updater"], response["updater"]);
        node.machine
            .as_mut()
            .unwrap()
            .updater
            .as_mut()
            .unwrap()
            .code = Some("SECRET".into());
        assert!(companion_status(&node).is_none());
    }

    #[tokio::test]
    async fn signed_fresh_connected_progress_settles_companion_only_retry_without_reconnect() {
        let f = orchestrator_fixture("companion_only_retry").await;
        let mut node = node(&f, &f.owner).await;
        node.metadata = Some(crate::models::node::NodeMetadata {
            agent_version: Some(updates::TARGET.into()),
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        });
        updates::begin(&f.state.db, &node, &f.owner, None, false, false)
            .await
            .unwrap();
        updates::observe(
            &f.state.db,
            &node,
            Some(&nyxid_machine::update::Progress {
                target: updates::TARGET.into(),
                phase: nyxid_machine::update::Phase::Connected,
                started_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                code: None,
                updater: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            updates::get(&f.state.db, &node).await.unwrap().phase,
            "connected"
        );
    }

    #[tokio::test]
    async fn updater_failure_codes_reach_status_tool_and_thread_with_guidance() {
        let f = orchestrator_fixture("machine_update_diagnostics").await;
        let node = node(&f, &f.owner).await;
        for code in [
            "verify_updater_image:trust_root_unavailable",
            "SECRET_UPSTREAM_BODY",
        ] {
            updates::begin(&f.state.db, &node, &f.owner, Some(&f.row.id), false, true)
                .await
                .unwrap();
            updates::observe(
                &f.state.db,
                &node,
                Some(&nyxid_machine::update::Progress {
                    updater: None,
                    target: updates::TARGET.into(),
                    phase: nyxid_machine::update::Phase::Failed,
                    started_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                    code: Some(code.into()),
                }),
            )
            .await
            .unwrap();
            let row = updates::get(&f.state.db, &node).await.unwrap();
            let expected = if code.starts_with("SECRET") {
                "update_failed_or_rolled_back"
            } else {
                code
            };
            assert_eq!(row.code.as_deref(), Some(expected));
            let response = serde_json::to_value(Status::new(&node, row.clone())).unwrap();
            assert_eq!(response["code"], expected);
            assert!(response["guidance"].is_string());
            let (tool_result, _) = tool(&f.state, &f.chat, &json!({"machine":node.id}))
                .await
                .unwrap();
            assert_eq!(tool_result["previous_update"]["code"], expected);
            let watch = f
                .state
                .db
                .collection::<NyxbotWatch>(WATCHES)
                .find_one(doc! {"connect_link_id":&row.attempt_id})
                .await
                .unwrap()
                .unwrap();
            super::super::nyxbot::machine_update_watch(&f.state, &watch)
                .await
                .unwrap();
            let conversation =
                crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
                    .await
                    .unwrap();
            let transcript = serde_json::to_string(&conversation).unwrap();
            assert!(transcript.contains(expected));
            assert!(!transcript.contains("SECRET_UPSTREAM_BODY"));
        }
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_specialists_need_grant_then_always_owner_card_and_guided_watch() {
        let mut f = fixture("machine_update_specialist").await;
        let node = node(&f, &f.owner).await;
        let args = json!({"machine":node.id});
        let (denied, _) = tool(&f.state, &f.chat, &args).await.unwrap();
        let card = f
            .state
            .db
            .collection::<AssistantAcknowledgement>(ACKS)
            .find_one(doc! {"_id":denied["acknowledgement_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.kind, "machine");
        f.chat.machine_node_ids.push(node.id.clone());
        let (result, _) = tool(&f.state, &f.chat, &args).await.unwrap();
        let id = result["acknowledgement_id"].as_str().unwrap();
        let card = f
            .state
            .db
            .collection::<AssistantAcknowledgement>(ACKS)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.kind, "action");
        acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        let (result, error) = tool(
            &f.state,
            &f.chat,
            &json!({"machine":node.id,"acknowledgement_id":id}),
        )
        .await
        .unwrap();
        assert!(!error);
        assert_eq!(result["status"], "manual_step");
        assert!(result.to_string().contains("End your turn"));
        for forbidden in ["nyx_nreg_", "nyx_nauth_", "signing_secret", "auth_token"] {
            assert!(!result.to_string().contains(forbidden));
        }
        let watch = f
            .state
            .db
            .collection::<NyxbotWatch>(WATCHES)
            .find_one(doc! {"kind":"machine_update","conversation_id":&f.row.id})
            .await
            .unwrap()
            .unwrap();
        let mut connected = node.clone();
        connected.connected_at = Some(chrono::Utc::now() + chrono::Duration::seconds(1));
        connected.metadata = Some(crate::models::node::NodeMetadata {
            agent_version: Some(updates::TARGET.into()),
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        });
        updates::observe(&f.state.db, &connected, None)
            .await
            .unwrap();
        crate::handlers::nyxbot::machine_update_watch(&f.state, &watch)
            .await
            .unwrap();
        let conversation =
            crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap();
        assert!(
            serde_json::to_value(conversation)
                .unwrap()
                .to_string()
                .contains("machine_update_finished")
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn update_signed_command_auto_idle_and_expiry_are_durable() {
        let f = orchestrator_fixture("machine_update_idle").await;
        let mut node = node(&f, &f.owner).await;
        node.machine.as_mut().unwrap().updater_ready = true;
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .replace_one(doc! {"_id": &node.id}, &node)
            .await
            .unwrap();
        let (task, mut seen) = peer(&f, &node, json!({"accepted":true})).await;
        let node = crate::services::node_service::get_node_by_id(&f.state.db, &node.id)
            .await
            .unwrap()
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.row.id},
                doc! {"$set":{"active_turn.machine_node_ids":[&node.id]}},
            )
            .await
            .unwrap();
        assert!(!updates::idle(&f.state.db, &node.id).await.unwrap());
        assert!(begin(&f.state, &node, &f.owner, None, true).await.is_err());
        assert!(seen.try_recv().is_err());
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.row.id},
                doc! {"$set":{"active_turn.machine_node_ids":[]}},
            )
            .await
            .unwrap();
        assert_eq!(
            begin(&f.state, &node, &f.owner, Some(&f.row.id), true)
                .await
                .unwrap()["status"],
            "queued"
        );
        let request = seen.recv().await.unwrap();
        assert_eq!(request.operation, Operation::Upgrade);
        assert_eq!(request.parameters["version"], updates::TARGET);
        assert_eq!(request.parameters["automatic"], true);
        f.state.db.collection::<bson::Document>(COLLECTION_NAME).update_one(doc!{"_id":&node.id},doc!{"$set":{"deadline":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
        let watch = f
            .state
            .db
            .collection::<NyxbotWatch>(WATCHES)
            .find_one(doc! {"kind":"machine_update"})
            .await
            .unwrap()
            .unwrap();
        crate::handlers::nyxbot::machine_update_watch(&f.state, &watch)
            .await
            .unwrap();
        assert_eq!(
            updates::get(&f.state.db, &node)
                .await
                .unwrap()
                .code
                .as_deref(),
            Some("update_reconnect_timeout")
        );
        task.abort();
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn migration_inspects_a_different_granted_host_before_owner_card() {
        TEST_UPDATER_IMAGE
            .scope(
                format!(
                    "{}@sha256:{}",
                    nyxid_machine::update::UPDATER_IMAGE,
                    "ab".repeat(32)
                ),
                async {
                    migration_with_verified_image(false).await;
                    migration_with_verified_image(true).await;
                },
            )
            .await;
    }

    async fn migration_with_verified_image(legacy: bool) {
        let mut f = fixture("machine_update_host").await;
        let mut target = node(&f, &f.owner).await;
        if legacy {
            target.machine.as_mut().unwrap().installation =
                Some(nyxid_machine::update::Installation::Container);
            target.machine.as_mut().unwrap().updater_ready = true;
            target.machine.as_mut().unwrap().updater =
                Some(nyxid_machine::update::CompanionStatus::legacy());
            f.state
                .db
                .collection::<Node>(crate::models::node::COLLECTION_NAME)
                .replace_one(doc! {"_id": &target.id}, &target)
                .await
                .unwrap();
        }
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &target.id},
                doc! {"$set": {"name": "target-machine"}},
            )
            .await
            .unwrap();
        let mut host = node(&f, &f.owner).await;
        host.metadata = Some(crate::models::node::NodeMetadata {
            agent_version: Some("0.41.4".into()),
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        });
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .replace_one(doc! {"_id": &host.id}, &host)
            .await
            .unwrap();
        // The host's shell grant is checked against live authority, not an
        // in-memory chat snapshot. Persist the fixture's legacy memberships.
        use crate::services::assistant_team_service as team;
        Box::pin(team::set_grants(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            team::GrantChange::Machine {
                base: Box::new(team::GrantChange::Add(Default::default())),
                machines: Some(vec![target.id.clone(), host.id.clone()]),
                logins: None,
                mode: team::MachineGrantMode::Add,
            },
        ))
        .await
        .unwrap();
        f.chat = acks::for_key(&f.state.db, &f.owner, Some(&f.chat.api_key_id))
            .await
            .unwrap()
            .unwrap();
        let (task, mut seen) = peer(
            &f,
            &host,
            json!({"docker_access":true,"container_id":"checked-id","companion_id":"updater-checked-id","accepted":true}),
        )
        .await;
        assert!(
            tool(
                &f.state,
                &f.chat,
                &json!({"machine":target.id,"host_machine":target.id,"container":"official"})
            )
            .await
            .is_err()
        );
        let args = json!({"machine":target.id,"host_machine":host.id,"container":"official"});
        let (card, _) = tool(&f.state, &f.chat, &args).await.unwrap();
        assert_eq!(
            seen.recv().await.unwrap().operation,
            Operation::ContainerInspect
        );
        assert!(seen.try_recv().is_err());
        let id = card["acknowledgement_id"].as_str().unwrap();
        acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        let mut approved = args;
        approved["acknowledgement_id"] = json!(id);
        let (result, _) = tool(&f.state, &f.chat, &approved).await.unwrap();
        assert_eq!(
            seen.recv().await.unwrap().operation,
            Operation::ContainerInspect
        );
        let migration = seen.recv().await.unwrap();
        assert_eq!(migration.operation, Operation::ContainerMigrate);
        assert_eq!(migration.parameters["container_id"], "checked-id");
        if legacy {
            assert_eq!(migration.parameters["replace_companion"], true);
            assert_eq!(migration.parameters["companion_id"], "updater-checked-id");
            assert!(
                updates::get(&f.state.db, &target)
                    .await
                    .unwrap()
                    .replace_companion
            );
        }
        assert_eq!(
            migration.parameters["updater_image"],
            TEST_UPDATER_IMAGE.with(Clone::clone)
        );
        assert_eq!(result["status"], "migrating");
        task.abort();
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn update_human_routes_refuse_delegates_and_execute_for_owner() {
        let f = orchestrator_fixture("machine_update_human").await;
        let mut node = node(&f, &f.owner).await;
        node.machine.as_mut().unwrap().updater_ready = true;
        f.state
            .db
            .collection::<Node>(crate::models::node::COLLECTION_NAME)
            .replace_one(doc! {"_id": &node.id}, &node)
            .await
            .unwrap();
        let human = crate::test_utils::test_auth_user(&f.owner);
        let mut oauth = human.clone();
        oauth.oauth_client_id = Some("third-party".into());
        for denied in [f.auth.clone(), oauth] {
            assert!(
                start(
                    State(f.state.clone()),
                    denied.clone(),
                    Path(node.id.clone())
                )
                .await
                .is_err()
            );
            assert!(
                policy(
                    State(f.state.clone()),
                    denied.clone(),
                    Path(node.id.clone()),
                    Json(Policy { automatic: true })
                )
                .await
                .is_err()
            );
            assert!(list(State(f.state.clone()), denied).await.is_err());
        }
        let _ = policy(
            State(f.state.clone()),
            human.clone(),
            Path(node.id.clone()),
            Json(Policy { automatic: true }),
        )
        .await
        .unwrap();
        let (task, mut seen) = peer(&f, &node, json!({"accepted":true})).await;
        let result = start(State(f.state.clone()), human, Path(node.id.clone()))
            .await
            .unwrap();
        assert_eq!(result.0["status"], "queued");
        assert_eq!(seen.recv().await.unwrap().operation, Operation::Upgrade);
        assert!(updates::get(&f.state.db, &node).await.unwrap().automatic);
        task.abort();
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn delayed_update_watches_keep_their_attempt_and_owner_transfer_resets_policy() {
        let f = orchestrator_fixture("machine_update_attempts").await;
        let mut node = node(&f, &f.owner).await;
        updates::policy(&f.state.db, &node, true).await.unwrap();
        let first = updates::begin(&f.state.db, &node, &f.owner, Some(&f.row.id), false, true)
            .await
            .unwrap();
        updates::finish(&f.state.db, &first, "failed", Some("old_failure"))
            .await
            .unwrap();
        let second = updates::begin(&f.state.db, &node, &f.owner, None, false, true)
            .await
            .unwrap();
        let saved = updates::watched(&f.state.db, first.attempt_id.as_deref().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.code.as_deref(), Some("old_failure"));
        assert_ne!(saved.attempt_id, second.attempt_id);
        node.user_id = uuid::Uuid::new_v4().to_string();
        updates::policy(&f.state.db, &node, false).await.unwrap();
        let current = updates::get(&f.state.db, &node).await.unwrap();
        assert!(!current.automatic);
        assert!(current.attempt_id.is_none());
        let prior = updates::watched(&f.state.db, second.attempt_id.as_deref().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prior.code.as_deref(), Some("machine_ownership_changed"));
        f.state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn update_reconnect_change_stream_wakes_the_original_thread_without_sweep() {
        use std::time::Duration;
        let f = orchestrator_fixture("machine_update_live").await;
        let node = node(&f, &f.owner).await;
        updates::begin(&f.state.db, &node, &f.owner, Some(&f.row.id), false, true)
            .await
            .unwrap();
        let live = f.state.assistant_live.clone();
        let db = f.state.db.clone();
        let runner = tokio::spawn(async move { live.run(db).await });
        let mut open = f.state.assistant_live.watch_open();
        tokio::time::timeout(Duration::from_secs(60), async {
            while !*open.borrow_and_update() {
                open.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        crate::handlers::nyxbot::spawn_live_dispatch(f.state.clone());
        f.state.db.collection::<Node>(crate::models::node::COLLECTION_NAME).update_one(
            doc! {"_id": &node.id},
            doc! {"$set": {
                "connected_at": bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::seconds(1)),
                "metadata.agent_version": updates::TARGET,
            }},
        ).await.unwrap();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let row =
                    crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
                        .await
                        .unwrap();
                let events = serde_json::to_string(&row.pending_events).unwrap();
                if events.contains("machine_update_finished") {
                    assert!(
                        events.contains("browser snapshot")
                            || events.contains("machine_browser snapshot")
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Update reconnect must wake via change stream");
        runner.abort();
        f.state.db.drop().await.unwrap();
    }
}
