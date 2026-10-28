use super::*;

fn root_aad(row: &ModelControlRoot) -> Vec<u8> {
    serde_json::to_vec(&(
        "nyxid:owner-model-control:v1",
        &row.id,
        &row.user_id,
        &row.conversation_id,
        &row.api_key_id,
        "private_ciphertext",
    ))
    .expect("context strings")
}
pub(super) fn step_aad(row: &ModelControlRoot, step: &str) -> Vec<u8> {
    serde_json::to_vec(&(
        "nyxid:owner-model-control:v1",
        &row.id,
        &row.user_id,
        &row.conversation_id,
        &row.api_key_id,
        "step_capability",
        step,
    ))
    .expect("context strings")
}

pub(super) fn new_token() -> AppResult<wire::SecretToken> {
    use rand::RngCore;
    let mut bytes = Zeroizing::new([0_u8; 32]);
    rand::rngs::OsRng.fill_bytes(bytes.as_mut());
    wire::SecretToken::parse(hex::encode(bytes.as_slice())).map_err(|_| invalid())
}

pub(super) async fn seal(
    state: &AppState,
    row: &ModelControlRoot,
    private: &RootPrivate,
) -> AppResult<Vec<u8>> {
    let mut bytes = Zeroizing::new(Vec::new());
    serde_json::to_writer(&mut *bytes, private).map_err(|_| invalid())?;
    if bytes.len() > MAX_PRIVATE_BYTES {
        return Err(unsupported());
    }
    state
        .encryption_keys
        .encrypt_with_aad(bytes.as_slice(), root_aad(row).as_slice())
        .await
}
pub(super) async fn open(state: &AppState, row: &ModelControlRoot) -> AppResult<RootPrivate> {
    let bytes = Zeroizing::new(
        state
            .encryption_keys
            .decrypt_with_aad(&row.private_ciphertext, root_aad(row).as_slice())
            .await?,
    );
    if bytes.len() > MAX_PRIVATE_BYTES {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

pub(super) fn require_row(row: &ModelControlRoot, chat: &acks::ChatAuthority) -> AppResult<()> {
    if chat.guest
        || !chat.turn_live
        || chat.turn_stopped
        || row.expires_at <= Utc::now()
        || row.user_id != chat.user_id
        || row.conversation_id != chat.conversation_id
        || row.api_key_id != chat.api_key_id
        || !chat
            .turn_id
            .as_ref()
            .is_some_and(|turn| row.turn_ids.contains(turn))
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) async fn load(
    state: &AppState,
    id: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<(ModelControlRoot, RootPrivate)> {
    let find = state
        .db
        .collection::<ModelControlRoot>(COLLECTION_NAME)
        .find_one(doc! {"_id":id});
    let row = match session {
        Some(session) => find.session(session).await?,
        None => find.await?,
    }
    .ok_or_else(invalid)?;
    let private = open(state, &row).await?;
    Ok((row, private))
}

pub(super) async fn authenticated(
    state: &AppState,
    chat: &acks::ChatAuthority,
    token: &wire::SecretToken,
    session: Option<&mut ClientSession>,
) -> AppResult<(ModelControlRoot, RootPrivate, BoundControl)> {
    let hash = token_hash(token.expose());
    let find = state
        .db
        .collection::<ModelControlRoot>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
            "api_key_id":&chat.api_key_id,"control_hashes":&hash,
        });
    let row = match session {
        Some(session) => find.session(session).await?,
        None => find.await?,
    }
    .ok_or_else(invalid)?;
    require_row(&row, chat)?;
    let private = open(state, &row).await?;
    let control = private
        .controls
        .iter()
        .find(|c| token_hash(c.token.expose()) == hash)
        .cloned()
        .ok_or_else(invalid)?;
    if control.claims.exp <= Utc::now().timestamp() as u64
        || control.claims.protocol != wire::PROTOCOL
        || control.claims.token_type != TOKEN_TYPE
        || control.claims.root_id != row.id
        || control.claims.runtime_catalog_id != row.runtime_catalog_id
        || control.claims.user_id != row.user_id
        || control.claims.conversation_id != row.conversation_id
        || control.claims.api_key_id != row.api_key_id
        || control.claims.aud != AUDIENCE
        || control.claims.iss != state.config.jwt_issuer
        || Some(control.claims.active_turn_id.as_str()) != chat.turn_id.as_deref()
    {
        return Err(invalid());
    }
    chat.ensure_live_turn(&state.db, None, None, "assistant.model_control")
        .await?;
    Ok((row, private, control))
}

pub(super) async fn save(
    state: &AppState,
    row: &mut ModelControlRoot,
    private: &RootPrivate,
    session: &mut ClientSession,
) -> AppResult<()> {
    let version = row.revision;
    row.private_ciphertext = seal(state, row, private).await?;
    row.revision = version.checked_add(1).ok_or_else(conflict)?;
    if state
        .db
        .collection::<ModelControlRoot>(COLLECTION_NAME)
        .replace_one(doc! {"_id":&row.id,"revision":version}, &*row)
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(conflict());
    }
    Ok(())
}

pub(super) async fn step_reply(
    state: &AppState,
    root: &ModelControlRoot,
    step: &Step,
) -> AppResult<wire::PrepareReply> {
    match &step.phase {
        StepPhase::Prepared => {
            let raw = Zeroizing::new(
                state
                    .encryption_keys
                    .decrypt_with_aad(
                        &step.capability_ciphertext,
                        step_aad(root, &step.id).as_slice(),
                    )
                    .await?,
            );
            let raw = std::str::from_utf8(&raw).map_err(|_| invalid())?.to_owned();
            let token = wire::SecretToken::parse(raw).map_err(|_| invalid())?;
            if token_hash(token.expose()) != step.capability_hash {
                return Err(invalid());
            }
            Ok(wire::PrepareReply::Prepared {
                step: wire::PreparedStep {
                    root_id: root.id.clone(),
                    step_id: step.id.clone(),
                    ordinal: step.ordinal,
                    request_binding: step.request_binding.clone(),
                    expires_at: root.expires_at.timestamp() as u64,
                    step_bearer: token,
                },
            })
        }
        StepPhase::Completed { receipt, .. } => Ok(wire::PrepareReply::Completed {
            receipt: receipt.clone(),
        }),
        StepPhase::Dispatched { .. } => Ok(wire::PrepareReply::InProgress {
            step_id: step.id.clone(),
        }),
        StepPhase::OutcomeUnknown { .. } => Ok(wire::PrepareReply::OutcomeUnknown),
    }
}

pub(super) fn verify_tool_results(
    step: &Step,
    receipts: &[wire::ToolResultReceipt],
) -> AppResult<Vec<VerifiedToolResult>> {
    if receipts.len() > wire::MAX_ITEMS {
        return Err(unsupported());
    }
    let mut results = Vec::new();
    for receipt in receipts {
        let saved = step
            .tools
            .iter()
            .find_map(|tool| match &tool.phase {
                HostToolPhase::Completed { receipt: saved }
                    if saved.call_id == receipt.call_id
                        && saved.tool_name == receipt.tool_name
                        && saved.arguments_binding == receipt.arguments_binding
                        && saved.result_binding == receipt.result_binding
                        && saved.server_activity_id == receipt.server_activity_id
                        && token_hash(saved.bearer.expose())
                            == token_hash(receipt.bearer.expose()) =>
                {
                    Some(saved)
                }
                _ => None,
            })
            .ok_or_else(conflict)?;
        results.push(VerifiedToolResult {
            call_id: receipt.call_id.clone(),
            tool_name: receipt.tool_name.clone(),
            arguments_binding: receipt.arguments_binding.clone(),
            model_text_binding: saved.result_binding.clone(),
        });
    }
    Ok(results)
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    use mongodb::{IndexModel, options::IndexOptions};
    let roots = db.collection::<ModelControlRoot>(COLLECTION_NAME);
    roots
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_id":1,"api_key_id":1,"turn_ids":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("model_control_turn_lineage_v1".into())
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"acknowledgement_id":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"acknowledgement_id":{"$type":"string"}})
                        .name("model_control_card_v1".into())
                        .build(),
                )
                .build(),
        ])
        .await?;
    // No TTL on roots: expiry revokes use, it does not erase retry tombstones.
    Ok(())
}
