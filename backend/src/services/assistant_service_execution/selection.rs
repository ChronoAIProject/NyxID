use super::*;

pub fn live_authority<'a>(
    db: &'a Database,
    original: &'a acks::ChatAuthority,
    route: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<acks::ChatAuthority>> {
    Box::pin(async move { reload_authority(db, original, route, None, false, true).await })
}

/// Personal legacy execution retains the default-off rollout contract. An
/// admission captured in a live turn always keeps its mandatory same-turn fence;
/// an idle snapshot cannot borrow a later turn or its selection/consent.
pub(crate) fn personal_authority<'a>(
    db: &'a Database,
    original: &'a acks::ChatAuthority,
    route: &'a str,
    connection: Option<&'a str>,
    require_service_grant: bool,
) -> futures::future::BoxFuture<'a, AppResult<acks::ChatAuthority>> {
    Box::pin(async move {
        reload_authority(
            db,
            original,
            route,
            connection,
            require_service_grant,
            false,
        )
        .await
    })
}

async fn reload_authority(
    db: &Database,
    original: &acks::ChatAuthority,
    route: &str,
    connection: Option<&str>,
    require_service_grant: bool,
    require_live: bool,
) -> AppResult<acks::ChatAuthority> {
    let key =
        crate::services::key_service::get_api_key(db, &original.user_id, &original.api_key_id)
            .await?;
    if key.expires_at.is_some_and(|at| at <= Utc::now()) {
        return Err(AppError::Forbidden("Assistant credential expired".into()));
    }
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        db, &key, None, None, None,
    ))
    .await?;
    // Only server-admitted model transport may retain the proxy's implicit
    // scope exception. Explicit grants still use the organization resource
    // boundary; ordinary MCP/native actions and targetless calls never skip it.
    let implicit_model_transport = !require_service_grant
        && !auth.allow_all_services
        && connection
            .is_some_and(|id| !auth.allowed_service_ids.iter().any(|allowed| allowed == id));
    if implicit_model_transport {
        let service = crate::services::user_service_service::find_user_service_by_id(
            db,
            connection.expect("implicit model transport has an exact connection"),
        )
        .await?
        .filter(|row| row.is_active)
        .ok_or_else(|| AppError::ApiKeyScopeForbidden("Model connection unavailable".into()))?;
        let catalog = service.catalog_service_id.as_deref().ok_or_else(|| {
            AppError::ApiKeyScopeForbidden("Model transport requires an inference catalog".into())
        })?;
        if db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .find_one(doc! {"_id":catalog,"is_active":true})
            .await?
            .is_none_or(|catalog| catalog.inference.is_none())
        {
            return Err(AppError::ApiKeyScopeForbidden(
                "Model transport scope revoked".into(),
            ));
        }
    } else {
        crate::services::org_agent_service::authorize_execution(db, &auth, connection).await?;
    }
    if require_service_grant
        && let Some(connection) = connection
        && !auth.allow_all_services
        && !auth.allowed_service_ids.iter().any(|id| id == connection)
    {
        return Err(AppError::ApiKeyScopeForbidden(
            "Service grant revoked".into(),
        ));
    }
    let live = acks::for_key(db, &original.user_id, Some(&original.api_key_id))
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    if live.conversation_id != original.conversation_id
        || live.agent_id != original.agent_id
        || live.turn_id != original.turn_id
        || live.invocation_id != original.invocation_id
        || live.invocation_controlled != original.invocation_controlled
        || if require_live || original.turn_live {
            !live.turn_live || live.turn_stopped
        } else {
            live.turn_live || live.turn_stopped != original.turn_stopped
        }
    {
        return Err(AppError::AssistantTurnRequired);
    }
    live.ensure_live_turn(db, None, None, route).await?;
    Ok(live)
}

pub(super) async fn selected(
    db: &Database,
    chat: &acks::ChatAuthority,
) -> AppResult<Option<Document>> {
    if !chat.turn_live || chat.turn_stopped {
        return Ok(None);
    }
    let mut filter = scope(chat)?;
    filter.insert("kind", "selection");
    Ok(db
        .collection::<Document>(COLLECTION)
        .find_one(filter)
        .await?)
}

pub(super) fn scope(chat: &acks::ChatAuthority) -> AppResult<Document> {
    if !chat.turn_live || chat.turn_stopped {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(
        doc! {"user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
        "api_key_id": &chat.api_key_id, "turn_id": chat.turn_id.as_deref().ok_or(AppError::AssistantTurnRequired)?,
        "expires_at": {"$gt": bson::DateTime::now()}},
    )
}

/// Closed syntax, interpreted only on admission of an authenticated owner's
/// message. Quoted text, model arguments and webhook input cannot select.
pub fn owner_selector(text: &str) -> Option<&str> {
    let first = text.lines().next()?;
    let value = first.strip_prefix("/service ")?.trim();
    (!value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_whitespace))
        .then_some(value)
}

/// Selection is distinct from consent and lasts for this one admitted turn.
pub async fn bind_owner_selection(
    db: &Database,
    session: &mut ClientSession,
    row: &crate::models::assistant_conversation::AssistantConversation,
    start: &crate::services::assistant_nyxagent::TurnStart,
    admitted_text: &str,
) -> AppResult<()> {
    use crate::models::assistant_conversation::TurnOrigin;
    let turn = row
        .active_turn
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    // Persisted by the admission transaction, independently of transcript role.
    // Events, assignments, groups and automations are legitimate model inputs;
    // their payloads never acquire human selection authority.
    db.collection::<Document>(COLLECTION)
        .insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "kind": "input_admission",
            "user_id": &row.user_id, "conversation_id": &row.id,
            "api_key_id": &row.credential_api_key_id, "turn_id": &turn.turn_id,
            "input_admission": digest(json!({"turn_id": turn.turn_id, "origin": start.origin,
                "text": admitted_text, "guest": start.guest})),
            "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(35)),
        })
        .session(&mut *session)
        .await?;
    if start.guest
        || start.trigger.is_some()
        || !matches!(start.origin, TurnOrigin::User | TurnOrigin::Channel)
    {
        return Ok(());
    }
    if let Some(id) = start.acknowledgement_id.as_deref() {
        let card = db.collection::<Document>("assistant_acknowledgements").find_one(doc! {
            "_id": id, "user_id": &row.user_id, "conversation_id": &row.id,
            "api_key_id": &row.credential_api_key_id, "tool_name": "nyxid.service_execution", "status": "allowed",
            "decided_by": "user", "expires_at": {"$gt": bson::DateTime::now()},
        }).session(&mut *session).await?;
        let binding = db.collection::<Document>(COLLECTION).find_one(doc! {"_id": id,
            "user_id": &row.user_id, "conversation_id": &row.id, "api_key_id": &row.credential_api_key_id,
            "expires_at": {"$gt": bson::DateTime::now()}}).session(&mut *session).await?;
        if let (Some(_), Some(mut binding)) = (card, binding) {
            let connection_id = binding
                .get_str("connection_id")
                .map_err(|_| AppError::Conflict("Invalid service confirmation".into()))?;
            if binding.get_str("binding")
                != Ok(
                    connection_binding(db, connection_id, &row.credential_api_key_id)
                        .await?
                        .as_str(),
                )
            {
                return Err(AppError::Conflict(
                    "Confirmed service changed; request a new confirmation".into(),
                ));
            }
            binding.insert("_id", uuid::Uuid::new_v4().to_string());
            binding.insert("kind", "selection");
            binding.insert("acknowledgement_id", id);
            binding.insert(
                "turn_id",
                &row.active_turn
                    .as_ref()
                    .ok_or(AppError::AssistantTurnRequired)?
                    .turn_id,
            );
            db.collection::<Document>(COLLECTION)
                .insert_one(binding)
                .session(session)
                .await?;
        }
        return Ok(());
    }
    let Some(reference) = owner_selector(&start.text) else {
        return Ok(());
    };
    let visible =
        crate::services::user_service_service::list_user_services_with_sources(db, &row.user_id)
            .await?;
    let choices: Vec<_> = visible
        .iter()
        .filter(|item| {
            !matches!(
                item.source,
                crate::services::user_service_service::CredentialSource::Org { allowed: false, .. }
            ) && (item.service.id == reference || item.service.slug == reference)
        })
        .collect();
    if choices.len() != 1 {
        return Err(AppError::ValidationError("Select one exact accessible connection UUID; this service name is missing or ambiguous".into()));
    }
    let turn = row
        .active_turn
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let connection = &choices[0].service;
    // A human selector chooses an identity, not an execution/credential grant.
    // In particular the conversation key can still be uncommitted here. Bind
    // owner and catalog identity now; concrete execution gets its full live
    // authority digest after this admission transaction commits.
    db.collection::<Document>(COLLECTION).insert_one(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "kind": "selection", "user_id": &row.user_id,
        "conversation_id": &row.id, "api_key_id": &row.credential_api_key_id, "turn_id": &turn.turn_id,
        "connection_id": &connection.id, "selection_source": "owner",
        "owner_id": &connection.user_id, "catalog_service_id": &connection.catalog_service_id,
        "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(35)),
    }).session(session).await?;
    Ok(())
}

/// Legacy catalog-only execution cannot escape an exact connection selected for
/// the same catalog, including a connection that has since been deleted.
pub(crate) async fn require_catalog_only_selection(
    db: &Database,
    chat: &acks::ChatAuthority,
    catalog: &str,
) -> AppResult<()> {
    if !chat.turn_live || chat.turn_stopped {
        return Ok(());
    }
    let mut filter = scope(chat)?;
    filter.extend(doc! {"kind":"selection","catalog_service_id":catalog});
    if db
        .collection::<Document>(COLLECTION)
        .find_one(filter)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Use the exact selected SSH connection; catalog fallback is unavailable".into(),
        ));
    }
    Ok(())
}
