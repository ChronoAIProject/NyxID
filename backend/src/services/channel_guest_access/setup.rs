use std::collections::HashSet;

use bson::doc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde::Serialize;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::{
    assistant_agent, channel_bot,
    channel_guest_policy::{
        COLLECTION_NAME as POLICIES, GuestAudience, GuestDataPolicy, GuestDataRule, GuestDataScope,
    },
    channel_guest_setup::{
        GuestSetup, PreparedSetup, SetupAudience, SetupConfiguration, SkillDeclaration,
    },
    nyxbot_channel, org_membership,
};
use crate::services::{
    audit_service::{self, AuditActor},
    org_membership_authority, transaction_service,
};

use super::{
    fence_selection_in_session, google_sheets, list_verified_actors, list_verified_rooms, policy,
    skills,
};

const MAX_GUESTS: usize = 32;
const MAX_RULES: usize = 32;
const MAX_DECLARATIONS: usize = 8;

pub(crate) async fn load(db: &Database, bot_id: &str) -> AppResult<Option<GuestSetup>> {
    Ok(db
        .collection(GuestSetup::COLLECTION_NAME)
        .find_one(doc! {"_id":bot_id})
        .await?)
}

fn admits(audience: Option<&SetupAudience>, guest_id: Option<&str>) -> bool {
    match audience {
        None | Some(SetupAudience::ChannelPolicy) => true,
        Some(SetupAudience::OwnerOnly) => false,
        Some(SetupAudience::Selected { guest_ids }) => {
            guest_id.is_some_and(|id| guest_ids.iter().any(|selected| selected == id))
        }
    }
}

pub(crate) async fn allows_guest(
    db: &Database,
    bot_id: &str,
    verified_guest_id: Option<&str>,
) -> AppResult<bool> {
    let setup = load(db, bot_id).await?;
    Ok(admits(
        setup
            .as_ref()
            .and_then(|row| row.applied.as_ref())
            .and_then(|config| config.audience.as_ref()),
        verified_guest_id,
    ))
}

fn conflict() -> AppError {
    AppError::Conflict("Guest setup changed. Reload the latest settings and try again.".into())
}

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}

pub(crate) fn validate_size(config: &SetupConfiguration) -> AppResult<()> {
    if config.rules.len() > MAX_RULES || config.declarations.len() > MAX_DECLARATIONS {
        return Err(invalid(
            "Guest setup has too many rules or skill declarations",
        ));
    }
    if bson::to_vec(config)
        .map_err(|_| invalid("Invalid guest setup"))?
        .len()
        > 48 * 1024
    {
        return Err(invalid("Guest setup exceeds its maximum size"));
    }
    Ok(())
}

async fn effective_rules(
    db: &Database,
    bot_id: &str,
    actor: &str,
    config: &SetupConfiguration,
    skill_rules: &[GuestDataRule],
) -> AppResult<(Vec<GuestDataRule>, Option<DeclarationBinding>)> {
    validate_size(config)?;
    let audience = config
        .audience
        .as_ref()
        .ok_or_else(|| invalid("Choose who may use this bot"))?;
    if let SetupAudience::Selected { guest_ids } = audience {
        let valid = !guest_ids.is_empty()
            && guest_ids.len() <= MAX_GUESTS
            && guest_ids.iter().collect::<HashSet<_>>().len() == guest_ids.len()
            && guest_ids.iter().all(|id| {
                !id.is_empty() && id.len() <= 128 && id.bytes().all(|byte| byte.is_ascii_graphic())
            });
        if !valid {
            return Err(invalid("Select one or more distinct verified guest IDs"));
        }
    }
    if matches!(audience, SetupAudience::OwnerOnly)
        && (!config.rules.is_empty() || !config.declarations.is_empty())
    {
        return Err(invalid(
            "Remove guest data rules before applying owner-only access",
        ));
    }

    let mut rules = config.rules.clone();
    let mut declaration_binding = None;
    if !config.declarations.is_empty() {
        let (binding, agent) = skills::maintainer_binding(db, actor, bot_id).await?;
        let mut seen = HashSet::new();
        for declaration in &config.declarations {
            if declaration.version != 1
                || !seen.insert(declaration.skill_id.as_str())
                || skills::exact_pin(&agent, declaration.skill_revision, &declaration.skill_id)
                    .is_none()
            {
                return Err(invalid(
                    "Use a version 1 declaration for a current, enabled bot skill pin",
                ));
            }
        }
        declaration_binding = Some(DeclarationBinding {
            agent_id: agent.id,
            binding_id: binding.id,
        });
    }
    rules.extend(skill_rules.iter().cloned());
    if rules.len() > MAX_RULES {
        return Err(invalid("Guest setup exceeds the number of effective rules"));
    }
    policy::validate_rules(&rules)?;
    if rules.iter().any(|rule| {
        rule.source_id == super::personal_records::SOURCE_ID
            && rule.operations.iter().any(|(operation, fields)| {
                matches!(operation_name(*operation), "create" | "update") && fields.write.is_empty()
            })
    }) {
        return Err(invalid(
            "Select at least one writable field for each Personal records create or update action",
        ));
    }
    let needs_guests = matches!(audience, SetupAudience::Selected { .. })
        || rules
            .iter()
            .any(|rule| !matches!(rule.audience, GuestAudience::AllGuests));
    let guests = if needs_guests {
        list_verified_actors(db, bot_id).await?
    } else {
        Vec::new()
    };
    if let SetupAudience::Selected { guest_ids } = audience
        && guest_ids.iter().any(|guest| !guests.contains(guest))
    {
        return Err(invalid("Select verified guests observed on this bot"));
    }
    let needs_rooms = rules.iter().any(|rule| {
        matches!(
            &rule.audience,
            GuestAudience::VerifiedAudience {
                room_id: Some(_),
                ..
            }
        )
    });
    let rooms = if needs_rooms {
        list_verified_rooms(db, bot_id).await?
    } else {
        Vec::new()
    };
    for rule in &rules {
        let observed = match &rule.audience {
            GuestAudience::AllGuests => true,
            GuestAudience::Guest { guest_id } => guests.contains(guest_id),
            GuestAudience::VerifiedAudience {
                guest_ids, room_id, ..
            } => {
                guest_ids.iter().all(|guest| guests.contains(guest))
                    && room_id.as_ref().is_none_or(|room| rooms.contains(room))
            }
        };
        if !observed {
            return Err(invalid(
                "The selected guest or room was not verified for this bot",
            ));
        }
    }
    if rules
        .iter()
        .any(|rule| rule.source_id == google_sheets::SOURCE_ID)
    {
        let eligible = sheets_connections(db, bot_id, actor).await?;
        for rule in rules
            .iter()
            .filter(|rule| rule.source_id == google_sheets::SOURCE_ID)
        {
            let GuestDataScope::Resources { record_ids } = &rule.scope else {
                return Err(invalid("Choose an exact Sheets resource"));
            };
            for operation in rule.operations.keys() {
                let action = operation_name(*operation);
                if !record_ids.iter().all(|resource| {
                    resource.split_once('/').is_some_and(|(id, _)| {
                        eligible
                            .iter()
                            .any(|row| row.id == id && row.actions.contains(&action))
                    })
                }) {
                    return Err(invalid(
                        "The selected Sheets connection and action must be granted to this bot",
                    ));
                }
            }
        }
    }
    Ok((rules, declaration_binding))
}

struct DeclarationBinding {
    agent_id: String,
    binding_id: String,
}

fn operation_name(
    operation: crate::models::channel_guest_policy::GuestDataOperation,
) -> &'static str {
    use crate::models::channel_guest_policy::GuestDataOperation::*;
    match operation {
        Create => "create",
        Read => "read",
        Update => "update",
        Delete => "delete",
    }
}

#[derive(Serialize)]
pub(crate) struct SheetsConnection {
    id: String,
    label: String,
    actions: Vec<&'static str>,
}

pub(crate) async fn sheets_connections(
    db: &Database,
    bot_id: &str,
    actor: &str,
) -> AppResult<Vec<SheetsConnection>> {
    use crate::models::{
        downstream_service, nyxbot_channel, user_api_key, user_endpoint, user_service,
    };
    use crate::services::{
        agent_operation_scope_service as scopes, org_channel_service as org_channels,
    };
    let agent = match skills::maintainer_agent(db, actor, bot_id).await {
        Ok(agent) if agent.kind == assistant_agent::AgentKind::Specialist => agent,
        Ok(_) | Err(AppError::Forbidden(_)) | Err(AppError::NotFound(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let channels: Vec<nyxbot_channel::NyxbotChannel> = db
        .collection(nyxbot_channel::COLLECTION_NAME)
        .find(doc! {"channel_bot_id":bot_id, "status":{"$in":["pending", "active"]}})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    let [channel] = channels.as_slice() else {
        return Ok(Vec::new());
    };
    let org_policy = channel
        .org_specialist
        .as_ref()
        .map(|binding| &binding.guests);
    if org_policy.is_some_and(|policy| !policy.enabled) {
        return Ok(Vec::new());
    }
    let org_ready = org_policy.is_some() && org_channels::guest_grants_ready(db).await?;
    let options = scopes::guest_options(db, actor, &agent.id).await?;
    let services: Vec<user_service::UserService> = db
        .collection(user_service::COLLECTION_NAME)
        .find(doc! {
            "_id":{"$in":&agent.grants.service_ids},
            "user_id":&agent.user_id,"is_active":true,"deleted_at":null,
            "node_id":null,"service_type":"http","admin_only":{"$ne":true},
        })
        .await?
        .try_collect()
        .await?;
    let catalog_ids: Vec<_> = services
        .iter()
        .filter_map(|service| service.catalog_service_id.as_deref())
        .collect();
    let catalogs: Vec<downstream_service::DownstreamService> = db
        .collection(downstream_service::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":catalog_ids},"is_active":true})
        .await?
        .try_collect()
        .await?;
    let mut result = Vec::new();
    for service in &services {
        let Some(catalog) = catalogs
            .iter()
            .find(|row| Some(row.id.as_str()) == service.catalog_service_id.as_deref())
        else {
            continue;
        };
        if !google_sheets::supports_catalog(&catalog.slug)
            || org_policy.is_some_and(|policy| {
                !org_channels::guest_service_selected(&agent, policy, org_ready, &service.id)
            })
        {
            continue;
        }
        let Some(option) = options
            .iter()
            .find(|option| option.service_id == service.id)
        else {
            continue;
        };
        let Some(key_id) = &service.api_key_id else {
            continue;
        };
        let credential = db
            .collection::<bson::Document>(user_api_key::COLLECTION_NAME)
            .find_one(doc! {
                "_id":key_id, "user_id":&agent.user_id, "credential_type":"oauth2",
                "$or":[
                    {"status":"active","access_token_encrypted":{"$type":"binData"},
                        "$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
                    {"status":{"$in":["active","expired"]},"refresh_token_encrypted":{"$type":"binData"}},
                ],
            })
            .projection(doc! {"_id":1})
            .await?;
        let endpoint = db
            .collection::<user_endpoint::UserEndpoint>(user_endpoint::COLLECTION_NAME)
            .find_one(doc! {"_id":&service.endpoint_id, "user_id":&agent.user_id})
            .await?;
        let Some(endpoint) = endpoint else {
            continue;
        };
        if credential.is_none() {
            continue;
        }
        let level = scopes::guest_level(&agent, &service.id, Some(&catalog.id));
        let mut actions = Vec::new();
        for operation in &option.operations {
            let Some(range) = operation
                .path
                .strip_prefix("/v4/spreadsheets/{spreadsheetId}/values/")
            else {
                continue;
            };
            let action = match (operation.method.as_str(), range) {
                ("GET", "{range}") => "read",
                ("POST", "{range}:append") if level != assistant_agent::GuestAccess::Read => {
                    "create"
                }
                _ => continue,
            };
            if !sheets_route_available(catalog, &endpoint.url, action) {
                continue;
            }
            let selectable = scopes::applicable(
                &agent.guest_operation_scopes,
                &service.id,
                Some(&catalog.id),
            )
            .chain(scopes::applicable(
                &agent.operation_scopes,
                &service.id,
                Some(&catalog.id),
            ))
            .all(|scope| {
                scope
                    .operations
                    .iter()
                    .any(|selected| selected.endpoint_id.as_deref() == Some(&operation.endpoint_id))
            });
            if selectable && !actions.contains(&action) {
                actions.push(action);
            }
        }
        if !actions.is_empty() {
            actions.sort_unstable_by_key(|action| *action != "read");
            result.push(SheetsConnection {
                id: service.id.clone(),
                label: endpoint.label,
                actions,
            });
        }
    }
    result.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
    Ok(result)
}

#[derive(Serialize)]
pub(crate) struct UnmappedGuestOperation {
    pub service: String,
    pub method: String,
    pub path: String,
}

pub(crate) async fn unmapped_guest_operations(
    db: &Database,
    bot_id: &str,
    actor: &str,
    sheets: &[SheetsConnection],
) -> AppResult<Vec<UnmappedGuestOperation>> {
    use crate::services::agent_operation_scope_service as scopes;
    let agent = match skills::maintainer_agent(db, actor, bot_id).await {
        Ok(agent) if agent.kind == assistant_agent::AgentKind::Specialist => agent,
        Ok(_) | Err(AppError::Forbidden(_)) | Err(AppError::NotFound(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut unsupported = Vec::new();
    for service in scopes::guest_options(db, actor, &agent.id).await? {
        let level = scopes::guest_level(&agent, &service.service_id, None);
        for operation in service.operations {
            if (!service.all_operations && !service.endpoint_ids.contains(&operation.endpoint_id))
                || (level == assistant_agent::GuestAccess::Read && !operation.read_only)
                || (level == assistant_agent::GuestAccess::Use && operation.changes_existing)
            {
                continue;
            }
            let supported_action = match (operation.method.as_str(), operation.path.as_str()) {
                ("GET", "/v4/spreadsheets/{spreadsheetId}/values/{range}") => Some("read"),
                ("POST", "/v4/spreadsheets/{spreadsheetId}/values/{range}:append") => {
                    Some("create")
                }
                _ => None,
            };
            if supported_action.is_some_and(|action| {
                sheets.iter().any(|connection| {
                    connection.id == service.service_id && connection.actions.contains(&action)
                })
            }) {
                continue;
            }
            unsupported.push(UnmappedGuestOperation {
                service: service.service_name.clone(),
                method: operation.method,
                path: operation.path,
            });
            if unsupported.len() == 50 {
                return Ok(unsupported);
            }
        }
    }
    Ok(unsupported)
}

fn sheets_route_available(
    catalog: &crate::models::downstream_service::DownstreamService,
    instance_origin: &str,
    action: &str,
) -> bool {
    use crate::services::{
        destination_routing::{select_target, workspace_destinations_pending},
        proxy_authorization::CanonicalPath,
    };
    if workspace_destinations_pending(catalog) {
        return false;
    }
    let (method, example) = match action {
        "read" => ("GET", "/v4/spreadsheets/example/values/Example!A1:B2"),
        "create" => ("POST", "/v4/spreadsheets/example/values/Example!A:B:append"),
        _ => return false,
    };
    let Ok(path) = CanonicalPath::from_rest_decoded(example) else {
        return false;
    };
    let Ok((_, target)) = select_target(catalog, method, &path) else {
        return false;
    };
    match target {
        Some(target) => catalog
            .destination_targets
            .get(&target)
            .is_some_and(|url| google_sheets::supports_destination(url)),
        None => google_sheets::supports_destination(instance_origin),
    }
}

async fn fence_manager(
    db: &Database,
    tx: &mut ClientSession,
    bot_id: &str,
    owner_id: &str,
    actor: &str,
    ownership_version: i64,
    mark_data: bool,
) -> AppResult<()> {
    org_membership_authority::fence_live_member(db, actor, tx).await?;
    if owner_id != actor {
        org_membership_authority::fence_org(db, owner_id, tx).await?;
        let membership = db.collection::<org_membership::OrgMembership>(org_membership::COLLECTION_NAME)
            .find_one(doc! {"org_user_id":owner_id,"member_user_id":actor,"role":"admin","revoked_at":null})
            .session(&mut *tx)
            .await?;
        if membership.is_none() {
            return Err(AppError::Forbidden(
                "A live organization admin is required".into(),
            ));
        }
    }
    let mut update = doc! {"$inc":{"guest_setup_write_revision":1_i64}};
    if mark_data {
        update.insert("$set", doc! {"channel_guest_data_present":true});
    }
    let bot = db
        .collection::<bson::Document>(channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":bot_id,"user_id":owner_id,"is_active":true,
            "$expr":{"$eq":[{"$ifNull":["$ownership_version",0_i64]},ownership_version]}},
            update,
        )
        .session(tx)
        .await?;
    if bot.matched_count != 1 {
        return Err(conflict());
    }
    Ok(())
}

async fn fence_agent_pin(
    db: &Database,
    tx: &mut ClientSession,
    bot_id: &str,
    binding_id: &str,
    agent_id: &str,
    declarations: &[SkillDeclaration],
) -> AppResult<()> {
    let revision = declarations[0].skill_revision;
    let revision_filter = if revision == 0 {
        doc! {"$in":[0_i64, bson::Bson::Null]}
    } else {
        doc! {"$eq":revision}
    };
    let agent = db
        .collection::<assistant_agent::AssistantAgent>(assistant_agent::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":agent_id,"destroyed_at":null,"skills_revision":revision_filter},
            doc! {"$inc":{"guest_declaration_fence":1_i64}},
        )
        .session(&mut *tx)
        .await?
        .ok_or_else(conflict)?;
    if declarations.iter().any(|declaration| {
        skills::exact_pin(&agent, declaration.skill_revision, &declaration.skill_id).is_none()
    }) {
        return Err(conflict());
    }
    let binding = db
        .collection::<bson::Document>(nyxbot_channel::COLLECTION_NAME)
        .update_one(
            declaration_binding_filter(bot_id, binding_id, &agent),
            doc! {"$inc":{"guest_declaration_fence":1_i64}},
        )
        .session(tx)
        .await?;
    if binding.matched_count != 1 {
        return Err(conflict());
    }
    Ok(())
}

fn declaration_binding_filter(
    bot_id: &str,
    binding_id: &str,
    agent: &assistant_agent::AssistantAgent,
) -> bson::Document {
    let mut filter = doc! {"_id":binding_id,"channel_bot_id":bot_id,"status":"active"};
    if agent.kind == assistant_agent::AgentKind::Nyxbot {
        filter.insert(
            "$or",
            vec![
                doc! {"agent_id":&agent.id},
                doc! {
                    "agent_id":null,"user_id":&agent.user_id,"org_specialist":null,
                },
            ],
        );
    } else {
        filter.insert("agent_id", &agent.id);
    }
    filter
}

pub(crate) struct SetupWrite<'a> {
    pub bot_id: &'a str,
    pub owner_id: &'a str,
    pub active: Option<(i64, i64)>,
    pub ownership_version: i64,
    pub expected_revision: i64,
    pub configuration: SetupConfiguration,
    pub actor: AuditActor,
    pub apply: bool,
    pub skill_rules: Vec<GuestDataRule>,
}

pub(crate) async fn save(db: &Database, write: SetupWrite<'_>) -> AppResult<()> {
    validate_size(&write.configuration)?;
    let effective = if write.apply {
        Some(
            effective_rules(
                db,
                write.bot_id,
                &write.actor.user_id,
                &write.configuration,
                &write.skill_rules,
            )
            .await?,
        )
    } else {
        None
    };
    let context = (db, write, effective);
    transaction_service::run_db_only(db, &context, |tx, context| {
        Box::pin(async move {
            let (db, write, effective) = context;
            if let Some((epoch, bot_revision)) = write.active {
                fence_selection_in_session(db, tx, write.bot_id, epoch, bot_revision).await?;
            }
            fence_manager(
                db,
                tx,
                write.bot_id,
                write.owner_id,
                &write.actor.user_id,
                write.ownership_version,
                true,
            )
            .await?;
            let setups = db.collection::<GuestSetup>(GuestSetup::COLLECTION_NAME);
            let current = setups
                .find_one(doc! {"_id":write.bot_id})
                .session(&mut *tx)
                .await?;
            if current.as_ref().map_or(0, |row| row.revision) != write.expected_revision
                || current
                    .as_ref()
                    .is_some_and(|row| row.owner_id != write.owner_id)
            {
                return Err(conflict());
            }
            let mut next = current.clone().unwrap_or_else(|| GuestSetup {
                bot_id: write.bot_id.to_owned(),
                owner_id: write.owner_id.to_owned(),
                revision: 0,
                draft: None,
                prepared: None,
                pending_activation: false,
                applied: None,
                applied_agent_id: None,
                applied_binding_id: None,
                policy_revision: 0,
                updated_at: chrono::Utc::now(),
            });
            next.revision = next.revision.checked_add(1).ok_or_else(conflict)?;
            next.updated_at = chrono::Utc::now();

            next.prepared = None;
            if let Some((rules, declaration_binding)) = effective {
                apply_configuration(
                    db,
                    tx,
                    &mut next,
                    &write.configuration,
                    rules,
                    declaration_binding.as_ref(),
                )
                .await?;
            } else {
                next.draft = Some(write.configuration.clone());
            }

            if let Some(previous) = current {
                let replaced = setups
                    .replace_one(
                        doc! {"_id":write.bot_id,"revision":previous.revision},
                        &next,
                    )
                    .session(&mut *tx)
                    .await?;
                if replaced.matched_count != 1 {
                    return Err(conflict());
                }
            } else {
                setups.insert_one(&next).session(&mut *tx).await?;
            }
            audit_service::log_actor_event_in_transaction(
                db,
                tx,
                &write.actor,
                if write.apply {
                    "channel_guest_setup_applied"
                } else {
                    "channel_guest_setup_draft_saved"
                },
                serde_json::json!({
                    "bot_id":write.bot_id,
                    "setup_revision":next.revision,
                    "policy_revision":next.policy_revision,
                }),
            )
            .await
        })
    })
    .await
}

async fn apply_configuration(
    db: &Database,
    tx: &mut ClientSession,
    setup: &mut GuestSetup,
    configuration: &SetupConfiguration,
    rules: &[GuestDataRule],
    binding: Option<&DeclarationBinding>,
) -> AppResult<()> {
    if let Some(binding) = binding {
        fence_agent_pin(
            db,
            tx,
            &setup.bot_id,
            &binding.binding_id,
            &binding.agent_id,
            &configuration.declarations,
        )
        .await?;
    }
    let policies = db.collection::<GuestDataPolicy>(POLICIES);
    let previous = policies
        .find_one(doc! {"bot_id":&setup.bot_id})
        .session(&mut *tx)
        .await?;
    let revision = previous
        .as_ref()
        .map_or(0, |policy| policy.revision)
        .checked_add(1)
        .ok_or_else(conflict)?;
    let policy = GuestDataPolicy {
        id: previous
            .as_ref()
            .map_or_else(|| Uuid::new_v4().to_string(), |policy| policy.id.clone()),
        bot_id: setup.bot_id.clone(),
        revision,
        rules: rules.to_vec(),
    };
    if let Some(previous) = previous {
        if policies
            .replace_one(
                doc! {"_id":previous.id,"revision":previous.revision},
                &policy,
            )
            .session(&mut *tx)
            .await?
            .matched_count
            != 1
        {
            return Err(conflict());
        }
    } else {
        policies.insert_one(&policy).session(&mut *tx).await?;
    }
    setup.policy_revision = revision;
    setup.draft = None;
    setup.prepared = None;
    setup.pending_activation = false;
    setup.applied = Some(configuration.clone());
    setup.applied_agent_id = binding.map(|binding| binding.agent_id.clone());
    setup.applied_binding_id = binding.map(|binding| binding.binding_id.clone());
    Ok(())
}

pub(crate) async fn set_preference(
    db: &Database,
    bot: &channel_bot::ChannelBot,
    actor: AuditActor,
    expected_revision: i64,
    enabled: bool,
) -> AppResult<()> {
    if bot.guest_access_v1_revision != expected_revision {
        return Err(conflict());
    }
    let mut candidate = bot.clone();
    candidate.guest_access_v1 = enabled;
    if bot.guest_access_v1 != enabled {
        candidate.guest_access_v1_revision =
            expected_revision.checked_add(1).ok_or_else(conflict)?;
    }
    let activation = if enabled
        && let Some(row) = load(db, &bot.id).await?
        && row.prepared.is_some()
        && let super::GuestMode::AddOn {
            activation_epoch, ..
        } = super::select_rollout_for_bot(db, &candidate).await?
    {
        match validate_prepared(db, &row).await {
            Ok(effective) => Some((activation_epoch, row, effective)),
            Err(error) if stale_preparation(&error) => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    if let Some(activation) = &activation {
        let outcome = write_preference(
            db,
            bot,
            &actor,
            expected_revision,
            enabled,
            Some(activation),
        )
        .await;
        match outcome {
            Ok(()) => return Ok(()),
            // A changed authority leaves the draft pending and the bot on its
            // ordinary path. The preference can still be saved independently.
            Err(error) if stale_preparation(&error) => {}
            Err(error) => return Err(error),
        }
    }
    write_preference(db, bot, &actor, expected_revision, enabled, None).await
}

type PreferenceActivation = (i64, GuestSetup, EffectiveRules);

async fn write_preference(
    db: &Database,
    bot: &channel_bot::ChannelBot,
    actor: &AuditActor,
    expected_revision: i64,
    enabled: bool,
    activation: Option<&PreferenceActivation>,
) -> AppResult<()> {
    let context = (db, bot, actor, expected_revision, enabled, activation);
    transaction_service::run_db_only(db, &context, |tx, context| {
        Box::pin(async move {
            let (db, bot, actor, expected_revision, enabled, activation) = context;
            fence_manager(
                db,
                tx,
                &bot.id,
                &bot.user_id,
                &actor.user_id,
                bot.ownership_version,
                false,
            )
            .await?;
            let revision = crate::services::channel_bot_service::set_guest_access_v1_in_session(
                db,
                tx,
                &bot.id,
                &bot.user_id,
                *expected_revision,
                *enabled,
            )
            .await?;
            if let Some((epoch, row, effective)) = activation {
                fence_selection_in_session(db, tx, &bot.id, *epoch, revision).await?;
                apply_prepared_in_session(db, tx, row, effective).await?;
            }
            if revision != *expected_revision {
                audit_service::log_actor_event_in_transaction(
                    db,
                    tx,
                    actor,
                    "channel_guest_preference_changed",
                    serde_json::json!({"bot_id":bot.id,"enabled":enabled,"bot_revision":revision}),
                )
                .await?;
            }
            Ok(())
        })
    })
    .await
}

pub(crate) async fn prepare(
    db: &Database,
    bot: &channel_bot::ChannelBot,
    actor: AuditActor,
    expected_revision: i64,
    skill_rules: Vec<GuestDataRule>,
    skill_selection: Option<skills::PreparedSelection>,
) -> AppResult<()> {
    let row = load(db, &bot.id)
        .await?
        .filter(|row| row.owner_id == bot.user_id && row.revision == expected_revision)
        .ok_or_else(conflict)?;
    let configuration = row
        .draft
        .as_ref()
        .ok_or_else(|| invalid("Save a guest setup draft first"))?;
    effective_rules(db, &bot.id, &actor.user_id, configuration, &skill_rules).await?;
    let prepared = PreparedSetup {
        draft_revision: expected_revision,
        maintainer_id: actor.user_id.clone(),
        ownership_version: bot.ownership_version,
        skill_rules,
        skill_selection,
    };
    let context = (db, bot, actor, prepared);
    transaction_service::run_db_only(db, &context, |tx, (db, bot, actor, prepared)| Box::pin(async move {
        fence_manager(db, tx, &bot.id, &bot.user_id, &actor.user_id, bot.ownership_version, true).await?;
        let next = prepared.draft_revision.checked_add(1).ok_or_else(conflict)?;
        let serialized = bson::to_bson(prepared).map_err(|_| AppError::Internal("Cannot store prepared guest setup".into()))?;
        let updated = db.collection::<GuestSetup>(GuestSetup::COLLECTION_NAME).update_one(
            doc! {"_id":&bot.id,"owner_id":&bot.user_id,"revision":prepared.draft_revision},
            doc! {"$set":{"prepared":serialized,"pending_activation":true,"revision":next,"updated_at":bson::DateTime::now()}},
        ).session(&mut *tx).await?;
        if updated.matched_count != 1 {
            return Err(conflict());
        }
        audit_service::log_actor_event_in_transaction(
            db, tx, actor, "channel_guest_setup_prepared",
            serde_json::json!({"bot_id":bot.id,"draft_revision":prepared.draft_revision}),
        ).await
    })).await
}

fn stale_preparation(error: &AppError) -> bool {
    matches!(
        error,
        AppError::ValidationError(_)
            | AppError::Forbidden(_)
            | AppError::Conflict(_)
            | AppError::NotFound(_)
            | AppError::ChannelBotNotFound(_)
            | AppError::OrgRoleInsufficient(_)
    )
}

type EffectiveRules = (Vec<GuestDataRule>, Option<DeclarationBinding>);

async fn validate_prepared(db: &Database, row: &GuestSetup) -> AppResult<EffectiveRules> {
    let prepared = row.prepared.as_ref().ok_or_else(conflict)?;
    let configuration = row.draft.as_ref().ok_or_else(conflict)?;
    effective_rules(
        db,
        &row.bot_id,
        &prepared.maintainer_id,
        configuration,
        &prepared.skill_rules,
    )
    .await
}

async fn apply_prepared_in_session(
    db: &Database,
    tx: &mut ClientSession,
    row: &GuestSetup,
    (rules, binding): &EffectiveRules,
) -> AppResult<()> {
    let collection = db.collection::<GuestSetup>(GuestSetup::COLLECTION_NAME);
    let current = collection
        .find_one(doc! {"_id":&row.bot_id})
        .session(&mut *tx)
        .await?;
    if current
        .as_ref()
        .is_some_and(|current| !current.pending_activation)
    {
        return Ok(());
    }
    if current.as_ref().map(|current| current.revision) != Some(row.revision) {
        return Err(conflict());
    }
    let prepared = row.prepared.as_ref().ok_or_else(conflict)?;
    let config = row.draft.as_ref().ok_or_else(conflict)?;
    if prepared.draft_revision.checked_add(1) != Some(row.revision) {
        return Err(conflict());
    }
    fence_manager(
        db,
        tx,
        &row.bot_id,
        &row.owner_id,
        &prepared.maintainer_id,
        prepared.ownership_version,
        true,
    )
    .await?;
    if let Some(selection) = &prepared.skill_selection {
        skills::apply_prepared_in_session(db, tx, &row.bot_id, selection).await?;
    }
    let mut next = row.clone();
    apply_configuration(db, tx, &mut next, config, rules, binding.as_ref()).await?;
    next.revision = next.revision.checked_add(1).ok_or_else(conflict)?;
    next.updated_at = chrono::Utc::now();
    let changed = collection
        .replace_one(doc! {"_id":&row.bot_id,"revision":row.revision}, &next)
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(conflict());
    }
    let audit_actor = AuditActor {
        user_id: prepared.maintainer_id.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    audit_service::log_actor_event_in_transaction(
        db, tx, &audit_actor, "channel_guest_setup_activated",
        serde_json::json!({"bot_id":row.bot_id,"setup_revision":next.revision,"policy_revision":next.policy_revision}),
    ).await
}

pub(crate) async fn activate_prepared_for_rollout(
    db: &Database,
    bot_id: &str,
    activation_epoch: i64,
    bot_revision: i64,
) -> AppResult<bool> {
    let Some(row) = load(db, bot_id).await? else {
        return Ok(true);
    };
    if row.prepared.is_none() {
        return Ok(!row.pending_activation);
    }
    let effective = match validate_prepared(db, &row).await {
        Ok(effective) => effective,
        Err(error) if stale_preparation(&error) => return Ok(false),
        Err(error) => return Err(error),
    };
    let context = (db, row, effective);
    let outcome = transaction_service::run_db_only(db, &context, |tx, (db, row, effective)| {
        Box::pin(async move {
            fence_selection_in_session(db, tx, &row.bot_id, activation_epoch, bot_revision).await?;
            apply_prepared_in_session(db, tx, row, effective).await
        })
    })
    .await;
    match outcome {
        Ok(()) => Ok(true),
        Err(error) if stale_preparation(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

fn applied_declarations(row: &GuestSetup, revision: i64) -> AppResult<&[SkillDeclaration]> {
    if row.policy_revision != revision {
        return Err(conflict());
    }
    Ok(row
        .applied
        .as_ref()
        .map_or(&[], |config| config.declarations.as_slice()))
}

pub(crate) async fn validate_applied_declarations(
    db: &Database,
    policy: &GuestDataPolicy,
) -> AppResult<()> {
    let Some(row) = load(db, &policy.bot_id).await? else {
        return Ok(());
    };
    let declarations = applied_declarations(&row, policy.revision)?;
    if declarations.is_empty() {
        return Ok(());
    }
    let agent_id = row.applied_agent_id.as_deref().ok_or_else(conflict)?;
    let binding_id = row.applied_binding_id.as_deref().ok_or_else(conflict)?;
    let agent = db
        .collection::<assistant_agent::AssistantAgent>(assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":agent_id,"destroyed_at":null})
        .await?
        .ok_or_else(conflict)?;
    let binding_live = db
        .collection::<bson::Document>(nyxbot_channel::COLLECTION_NAME)
        .find_one(declaration_binding_filter(
            &policy.bot_id,
            binding_id,
            &agent,
        ))
        .projection(doc! {"_id":1})
        .await?
        .is_some();
    if binding_live
        && declarations.iter().all(|declaration| {
            skills::exact_pin(&agent, declaration.skill_revision, &declaration.skill_id).is_some()
        })
    {
        Ok(())
    } else {
        Err(conflict())
    }
}

pub(crate) async fn fence_applied_declarations_in_session(
    db: &Database,
    tx: &mut ClientSession,
    policy: &GuestDataPolicy,
) -> AppResult<()> {
    let row = db
        .collection::<GuestSetup>(GuestSetup::COLLECTION_NAME)
        .find_one(doc! {"_id":&policy.bot_id})
        .session(&mut *tx)
        .await?;
    if let Some(row) = row {
        let declarations = applied_declarations(&row, policy.revision)?;
        if !declarations.is_empty() {
            fence_agent_pin(
                db,
                tx,
                &policy.bot_id,
                row.applied_binding_id.as_deref().ok_or_else(conflict)?,
                row.applied_agent_id.as_deref().ok_or_else(conflict)?,
                declarations,
            )
            .await?;
        }
    }
    Ok(())
}

pub(crate) async fn purge_bot(db: &Database, bot_id: &str) -> AppResult<()> {
    db.collection::<GuestDataPolicy>(POLICIES)
        .delete_many(doc! {"bot_id":bot_id})
        .await?;
    db.collection::<GuestSetup>(GuestSetup::COLLECTION_NAME)
        .delete_one(doc! {"_id":bot_id})
        .await?;
    Ok(())
}

/// Erase configuration for a proven exact current installation.
pub(crate) async fn purge_bot_in_session(
    db: &Database,
    tx: &mut ClientSession,
    bot_id: &str,
) -> AppResult<()> {
    db.collection::<GuestDataPolicy>(POLICIES)
        .delete_many(doc! {"bot_id":bot_id})
        .session(&mut *tx)
        .await?;
    db.collection::<GuestSetup>(GuestSetup::COLLECTION_NAME)
        .delete_one(doc! {"_id":bot_id})
        .session(tx)
        .await?;
    Ok(())
}

/// Keep the policy revision monotonic across ownership changes so a turn
/// admitted under the former owner can never match the new owner's policy.
pub(crate) async fn retire_for_transfer_in_session(
    db: &Database,
    tx: &mut ClientSession,
    bot_id: &str,
) -> AppResult<()> {
    db.collection::<GuestDataPolicy>(POLICIES)
        .update_one(
            doc! {"bot_id":bot_id},
            doc! {
                "$set":{"rules":[]},
                "$inc":{"revision":1_i64},
                "$setOnInsert":{"_id":Uuid::new_v4().to_string(),"bot_id":bot_id},
            },
        )
        .upsert(true)
        .session(&mut *tx)
        .await?;
    db.collection::<GuestSetup>(GuestSetup::COLLECTION_NAME)
        .delete_one(doc! {"_id":bot_id})
        .session(tx)
        .await?;
    Ok(())
}

#[derive(Serialize)]
pub(crate) struct Preview {
    pub basis: &'static str,
    pub guests_admitted: [bool; 2],
    pub rule_count: usize,
    pub checks: Vec<PreviewCheck>,
}

#[derive(Serialize)]
pub(crate) struct PreviewCheck {
    pub rule: usize,
    pub source_id: String,
    pub action: String,
    pub guest_a: bool,
    pub guest_b: bool,
    pub guest_b_on_a: Option<bool>,
    pub guest_a_in_group: bool,
    pub resource: Option<String>,
    pub read_fields: Vec<String>,
    pub write_fields: Vec<String>,
}

pub(crate) async fn preview_configuration(
    db: &Database,
    bot_id: &str,
    actor: &str,
    config: &SetupConfiguration,
    guest_ids: Option<[String; 2]>,
    skill_rules: &[GuestDataRule],
) -> AppResult<Preview> {
    let (rules, _) = effective_rules(db, bot_id, actor, config, skill_rules).await?;
    if let Some(ids) = &guest_ids {
        let verified = list_verified_actors(db, bot_id).await?;
        if ids.iter().any(|id| !verified.contains(id)) {
            return Err(invalid("Preview guest IDs must be verified on this bot"));
        }
    }
    preview_rules(config, &rules, guest_ids)
}

fn preview_rules(
    config: &SetupConfiguration,
    rules: &[GuestDataRule],
    guest_ids: Option<[String; 2]>,
) -> AppResult<Preview> {
    let samples = match config.audience.as_ref() {
        Some(SetupAudience::Selected { guest_ids }) => [
            guest_ids[0].clone(),
            guest_ids
                .get(1)
                .cloned()
                .unwrap_or_else(|| "setup-preview-guest-b".into()),
        ],
        _ => {
            let mut named = Vec::new();
            for rule in rules {
                match &rule.audience {
                    GuestAudience::Guest { guest_id } => named.push(guest_id),
                    GuestAudience::VerifiedAudience { guest_ids, .. } => named.extend(guest_ids),
                    GuestAudience::AllGuests => {}
                }
            }
            let mut seen = HashSet::new();
            named.retain(|id| seen.insert(*id));
            [
                named
                    .first()
                    .map_or_else(|| "setup-preview-guest-a".into(), |id| (*id).clone()),
                named
                    .get(1)
                    .map_or_else(|| "setup-preview-guest-b".into(), |id| (*id).clone()),
            ]
        }
    };
    let [guest_a, guest_b] = guest_ids.unwrap_or(samples);
    if guest_a.is_empty() || guest_b.is_empty() || guest_a == guest_b {
        return Err(invalid("Preview requires two distinct guest identities"));
    }
    let guests_admitted = [
        admits(config.audience.as_ref(), Some(&guest_a)),
        admits(config.audience.as_ref(), Some(&guest_b)),
    ];
    let mut checks = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        for (&operation, fields) in &rule.operations {
            let action = operation_name(operation);
            let create = action == "create";
            let own = matches!(rule.scope, GuestDataScope::OwnRecords);
            let records: Vec<_> = match &rule.scope {
                GuestDataScope::OwnRecords => vec![Some("sample-personal-record")],
                GuestDataScope::Resources { record_ids } => {
                    record_ids.iter().map(|id| Some(id.as_str())).collect()
                }
                GuestDataScope::Collection => vec![None],
            };
            let requested_fields: Vec<_> = match action {
                "create" | "update" => fields.write.iter().map(String::as_str).collect(),
                "read" => fields.read.iter().map(String::as_str).collect(),
                _ => Vec::new(),
            };
            for record in records {
                let allowed = |guest_id, owner, private| {
                    policy::preview(
                        rules,
                        &policy::GuestPolicySubject {
                            guest_id: Some(guest_id),
                            room_id: match &rule.audience {
                                GuestAudience::VerifiedAudience { room_id, .. } => {
                                    room_id.as_deref()
                                }
                                _ => None,
                            },
                            private,
                            // Simulate a supported verified room; live turns
                            // take this evidence from authenticated ingress.
                            shared_room_verified: !private,
                        },
                        &policy::GuestDataRequest {
                            source_id: &rule.source_id,
                            collection_id: &rule.collection_id,
                            action,
                            record_id: if own && create { None } else { record },
                            record_owner_id: if own && !create { Some(owner) } else { None },
                            fields: &requested_fields,
                        },
                    )
                    .allowed
                };
                checks.push(PreviewCheck {
                    rule: index + 1,
                    source_id: rule.source_id.clone(),
                    action: action.to_owned(),
                    guest_a: guests_admitted[0] && allowed(&guest_a, &guest_a, true),
                    guest_b: guests_admitted[1] && allowed(&guest_b, &guest_b, true),
                    guest_b_on_a: (own && !create)
                        .then(|| guests_admitted[1] && allowed(&guest_b, &guest_a, true)),
                    guest_a_in_group: guests_admitted[0] && allowed(&guest_a, &guest_a, false),
                    resource: (!own)
                        .then_some(record)
                        .flatten()
                        .and_then(|resource| resource.splitn(3, '/').nth(2))
                        .map(str::to_owned),
                    read_fields: fields.read.iter().cloned().collect(),
                    write_fields: fields.write.iter().cloned().collect(),
                });
            }
        }
    }
    Ok(Preview {
        basis: "simulated_records_policy_only",
        guests_admitted,
        rule_count: rules.len(),
        checks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_guest_setup_admission_distinguishes_owner_only_selected_and_unconfigured() {
        assert!(admits(None, None));
        assert!(!admits(Some(&SetupAudience::OwnerOnly), Some("guest-a")));
        let selected = SetupAudience::Selected {
            guest_ids: vec!["guest-a".into()],
        };
        assert!(admits(Some(&selected), Some("guest-a")));
        assert!(!admits(Some(&selected), Some("guest-b")));
        assert!(!admits(Some(&selected), None));
    }
}
