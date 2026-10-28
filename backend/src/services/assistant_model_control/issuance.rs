use super::*;
use crate::models::{
    assistant_acknowledgement::AssistantAcknowledgement,
    assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
};

/// Constructed only by NyxID's owner runtime request, after negotiating the
/// registered runtime's v1 capability. Neither a raw key nor a client header
/// can choose a root, continuation, input snapshot or next logical step.
pub(crate) struct IssueRequest<'a> {
    pub chat: &'a acks::ChatAuthority,
    pub runtime_catalog_id: &'a str,
    pub outer_body: &'a Value,
    pub negotiated_v1: bool,
}

pub(crate) async fn issue(
    state: &AppState,
    request: IssueRequest<'_>,
) -> AppResult<Option<TargetedControl>> {
    let IssueRequest {
        chat,
        runtime_catalog_id,
        outer_body,
        negotiated_v1,
    } = request;
    if chat.guest || chat.invocation_controlled {
        return Ok(None);
    }
    let turn_id = chat.turn_id.as_deref().ok_or_else(invalid)?;
    let messages = state.db.collection::<AssistantMessage>(MESSAGES);
    let input = messages
        .find_one(doc! {
            "conversation_id":&chat.conversation_id,"user_id":&chat.user_id,
            "turn_id":turn_id,"role":"user",
        })
        .await?;
    let roots = state.db.collection::<ModelControlRoot>(COLLECTION_NAME);
    let mut filter = doc! {"user_id":&chat.user_id,"conversation_id":&chat.conversation_id};
    // A saved decision or Retry carries its original lineage. A new turn may
    // not erase a dispatched/uncertain root by simply omitting its old token.
    if let Some(card) = input.as_ref().and_then(|i| i.continuation_of.as_ref()) {
        filter.insert("acknowledgement_id", card);
    } else {
        filter.insert(
            "turn_ids",
            input
                .as_ref()
                .and_then(|i| i.retry_of.as_deref())
                .unwrap_or(turn_id),
        );
        if input.as_ref().is_none_or(|i| i.retry_of.is_none()) {
            filter.insert("api_key_id", &chat.api_key_id);
        }
    }
    let existing = roots.find_one(filter).await?;
    // Resolve saved lineage BEFORE testing the new key. Rotation invalidates
    // that lineage; it must never look like a new, uncontrolled legacy action.
    if existing
        .as_ref()
        .is_some_and(|r| r.api_key_id != chat.api_key_id)
    {
        return Err(invalid());
    }
    if !state.config.assistant_model_control_v1_enabled || !negotiated_v1 {
        if existing.as_ref().is_some_and(|r| r.phase != "unactivated") {
            return Err(invalid());
        }
        return Ok(None);
    }
    let Some(input) = input else {
        // Event/trigger owner turns without a saved user-input continuation
        // keep their exact-only contract. No synthetic user input is minted.
        if existing.as_ref().is_some_and(|r| r.phase != "unactivated") {
            return Err(invalid());
        }
        return Ok(None);
    };
    super::super::security_readiness_service::require(&state.db, READINESS_CAPABILITY).await?;
    let binding = digest(outer_body.clone());
    let resumed_input = match outer_text(outer_body) {
        Ok(text) => text,
        Err(_) if existing.as_ref().is_none_or(|r| r.phase == "unactivated") => return Ok(None),
        Err(error) => return Err(error),
    };
    let now = Utc::now();
    let mut session = state.db.client().start_session().await?;
    let token = session.start_transaction().and_run2(async |session| {
        let result = async {
            let (live, _, _) = acks::fence(&state.db, chat, session).await?;
            super::super::assistant_access_policy::require_same_live_turn(chat, &live)?;
            let (mut row, mut private, new) = if let Some(expected) = &existing {
                let (row, private) = persistence::load(state, &expected.id, Some(&mut *session)).await?;
                (row, private, false)
            } else {
                // An unrelated old card never upgrades into reusable consent.
                if input.continuation_of.is_some() || input.retry_of.is_some() { return Ok(None); }
                (ModelControlRoot {
                    id:uuid::Uuid::new_v4().to_string(),user_id:chat.user_id.clone(),
                    conversation_id:chat.conversation_id.clone(),api_key_id:chat.api_key_id.clone(),
                    turn_ids:vec![turn_id.into()],runtime_catalog_id:runtime_catalog_id.into(),
                    phase:"unactivated".into(),acknowledgement_id:None,card_arguments:None,
                    revision:0,control_hashes:vec![],private_ciphertext:vec![],created_at:now,
                    expires_at:now+Duration::minutes(35),
                },RootPrivate {controls:vec![],envelope:None,initial_input:vec![],
                    connection_id:None,connection_binding:None,owner_id:None,catalog_id:None,
                    card_arguments:None,effective_headers_binding:None,steps:vec![]},true)
            };
            if row.user_id != chat.user_id || row.conversation_id != chat.conversation_id
                || row.api_key_id != chat.api_key_id
                || row.expires_at <= Utc::now() || row.runtime_catalog_id != runtime_catalog_id
                || matches!(row.phase.as_str(),"denied"|"revoked")
            { return Err(invalid()); }
            if let Some(bound) = private.controls.iter().find(|c|
                c.claims.active_turn_id == turn_id && c.claims.outer_request_binding == binding)
            { return Ok(Some(bound.token.clone())); }
            if private.controls.len() >= MAX_STEPS { return Err(unsupported()); }
            let origin = if let Some(ack) = input.continuation_of.as_deref() {
                let card = state.db.collection::<AssistantAcknowledgement>("assistant_acknowledgements")
                    .find_one(doc! {"_id":ack,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
                        "api_key_id":&chat.api_key_id,"tool_name":"nyxid.model_control_v1"})
                    .session(&mut *session).await?.ok_or_else(invalid)?;
                if row.acknowledgement_id.as_deref() != Some(ack) || row.phase != "allowed"
                    || !matches!(card.status.as_str(),"allowed"|"used") || card.decider != "user"
                    || card.decided_by.as_deref() != Some("user") || card.expires_at <= now
                { return Err(invalid()); }
                let first = private.controls.first().ok_or_else(invalid)?;
                let wire::ControlOrigin::Initial {input_id,creating_turn_id} = &first.claims.origin else { return Err(invalid()); };
                wire::ControlOrigin::AllowedContinuation {acknowledgement_id:ack.into(),
                    saved_continuation_id:input.id.clone(),original_input_id:input_id.clone(),original_turn_id:creating_turn_id.clone()}
            } else if let Some(old) = &input.retry_of {
                if !row.turn_ids.contains(old) { return Err(invalid()); }
                wire::ControlOrigin::Retry {original_turn_id:old.clone(),saved_continuation_id:input.id.clone()}
            } else {
                wire::ControlOrigin::Initial {input_id:input.id.clone(),creating_turn_id:turn_id.into()}
            };
            let claims = wire::ControlClaims {iss:state.config.jwt_issuer.clone(),aud:AUDIENCE.into(),
                token_type:TOKEN_TYPE.into(),protocol:wire::PROTOCOL.into(),jti:uuid::Uuid::new_v4().to_string(),
                iat:now.timestamp() as u64,exp:row.expires_at.timestamp() as u64,
                user_id:row.user_id.clone(),conversation_id:row.conversation_id.clone(),api_key_id:row.api_key_id.clone(),
                root_id:row.id.clone(),active_turn_id:turn_id.into(),origin,runtime_catalog_id:runtime_catalog_id.into(),
                outer_request_binding:binding.clone()};
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
            header.kid = Some(state.jwt_keys.kid.clone());
            let token = wire::SecretToken::parse(jsonwebtoken::encode(&header,&claims,&state.jwt_keys.encoding)
                .map_err(|_| invalid())?).map_err(|_| invalid())?;
            row.control_hashes.push(token_hash(token.expose()));
            if !row.turn_ids.iter().any(|id| id == turn_id) { row.turn_ids.push(turn_id.into()); }
            private.controls.push(BoundControl {claims,token:token.clone(),resumed_input:resumed_input.clone()});
            if new {
                row.private_ciphertext = persistence::seal(state,&row,&private).await?;
                roots.insert_one(&row).session(&mut *session).await?;
            } else {
                persistence::save(state,&mut row,&private,session).await?;
            }
            Ok(Some(token))
        }.await;
        super::super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)?;
    Ok(token.map(|token| TargetedControl {
        token,
        runtime_catalog_id: runtime_catalog_id.into(),
    }))
}

fn outer_text(body: &Value) -> AppResult<String> {
    if let Some(input) = body["input"].as_str() {
        return Ok(input.into());
    }
    let items = body["input"].as_array().ok_or_else(unsupported)?;
    if items.len() != 1 || items[0]["role"] != "user" {
        return Err(unsupported());
    }
    let parts = items[0]["content"].as_array().ok_or_else(unsupported)?;
    if parts.len() != 1 || parts[0]["type"] != "input_text" {
        return Err(unsupported());
    }
    parts[0]["text"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(unsupported)
}
