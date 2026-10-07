//! Human web steering. No additional SSE protocol; receipts use normal history.
use super::*;
use crate::services::assistant_steering::{self as steering, Admission};

#[derive(Clone, Serialize)]
pub struct SteerCapability {
    max_chars: usize,
    max_per_turn: usize,
}
#[derive(Serialize)]
pub struct CapabilitiesResponse {
    steer: Option<SteerCapability>,
}
fn steer_capability(value: &Value) -> Option<SteerCapability> {
    let cap = &value["steer"];
    if cap["version"] != 1
        || cap["protocol"] != "nyxagent-steer-v1"
        || !cap["input"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v == "text"))
    {
        return None;
    }
    Some(SteerCapability {
        max_chars: usize::try_from(cap["max_chars"].as_u64()?)
            .ok()?
            .min(steering::MAX_CHARS),
        max_per_turn: usize::try_from(cap["max_per_turn"].as_u64()?).ok()?.min(20),
    })
    .filter(|c| c.max_chars > 0 && c.max_per_turn > 0)
}

type CapabilityCache = HashMap<String, (Instant, Value)>;
static CAPABILITIES: LazyLock<Mutex<CapabilityCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One bounded metadata cache for image input and steering; never cache authority
/// or credentials. Include the destination so catalog changes cannot reuse support.
pub(super) async fn upstream_capabilities(
    state: &AppState,
    auth: &AuthUser,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
) -> Value {
    let Ok(service) =
        assistant_service::resolve_admin_service_by_slug(&state.db, engine::SERVICE_SLUG).await
    else {
        return Value::Null;
    };
    let key = format!("{}:{}:{}", state.db.name(), service.id, service.base_url);
    let mut cache = CAPABILITIES.lock().await;
    if let Some((at, value)) = cache.get(&key)
        && at.elapsed() < Duration::from_secs(60)
    {
        return value.clone();
    }
    let lookup = async {
        let response = Box::pin(proxy(
            state,
            auth,
            credential,
            "GET",
            "v1/capabilities",
            None,
            None,
            policy,
        ))
        .await
        .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .ok()?;
        serde_json::from_slice::<Value>(&bytes).ok()
    };
    let value = tokio::time::timeout(Duration::from_secs(5), lookup)
        .await
        .ok()
        .flatten()
        .unwrap_or(Value::Null);
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), value.clone()));
    value
}

pub async fn capabilities(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    request: Request<Body>,
) -> AppResult<Json<CapabilitiesResponse>> {
    super::super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let row = engine::get(&state.db, &owner, &id).await?;
    if row.guest_turn
        || row.group_id.is_some()
        || row.voice_parent_conversation_id.is_some()
        || row.channel.is_some()
    {
        return Ok(Json(CapabilitiesResponse { steer: None }));
    }
    let Some(credential) =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, &owner, &id).await?
    else {
        return Ok(Json(CapabilitiesResponse { steer: None }));
    };
    let value = upstream_capabilities(
        &state,
        &auth,
        &credential,
        request.extensions().get::<BillingRoutePolicy>().copied(),
    )
    .await;
    Ok(Json(CapabilitiesResponse {
        steer: steer_capability(&value),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteerRequest {
    text: String,
    turn_id: String,
    #[serde(rename = "clientRequestId")]
    client_request_id: String,
}

#[derive(Serialize)]
pub struct SteeringResponse {
    client_request_id: String,
    outcome: String,
    code: Option<String>,
}
impl From<&crate::models::assistant_message::Steering> for SteeringResponse {
    fn from(row: &crate::models::assistant_message::Steering) -> Self {
        Self {
            client_request_id: row.client_request_id.clone(),
            outcome: row.outcome.clone(),
            code: row.code.clone(),
        }
    }
}

fn refusal(code: &str) -> Response {
    let (status, message) = match code {
        "starting" => (
            409,
            "The assistant is starting. Try this guidance again shortly.",
        ),
        "no_active_turn" => (
            409,
            "This turn has ended. You can send the guidance as a new message.",
        ),
        "stop_pending" => (409, "The assistant is stopping. Wait for it to finish."),
        "response_mismatch" => (
            409,
            "The running response changed. Refresh the turn before sending guidance.",
        ),
        "idempotency_conflict" => (
            409,
            "This request ID was already used for different guidance.",
        ),
        "steer_limit" => (
            429,
            "This reply has reached its guidance limit. Wait for it to finish.",
        ),
        "invalid_request" => (
            400,
            "Guidance must be nonblank text of at most 32,000 characters with a valid request ID.",
        ),
        "not_found" => (404, "The assistant conversation was not found."),
        "steer_unsupported" => (409, "Steering is not supported for this turn."),
        _ => (
            503,
            "This guidance may not have been applied. Do not resend it automatically.",
        ),
    };
    let mut response = (
        StatusCode::from_u16(status).unwrap(),
        Json(json!({"error":code,"message":message})),
    )
        .into_response();
    if code == "starting" {
        response
            .headers_mut()
            .insert("retry-after", "1".parse().unwrap());
    }
    response
}
fn receipt_response(message: &AssistantMessage) -> Response {
    let receipt = message.steering.as_deref().expect("steering receipt");
    if receipt.http_status == 200 {
        Json(SteeringResponse::from(receipt)).into_response()
    } else {
        refusal(receipt.code.as_deref().unwrap_or("steer_unavailable"))
    }
}

pub async fn steer(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    request: Request<Body>,
) -> AppResult<Response> {
    super::super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let row = engine::get(&state.db, &owner, &id).await?;
    let policy = request.extensions().get::<BillingRoutePolicy>().copied();
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), engine::MAX_REQUEST_BYTES).await
    else {
        return Ok(refusal("invalid_request"));
    };
    let Ok(body) = serde_json::from_slice::<SteerRequest>(&bytes) else {
        return Ok(refusal("invalid_request"));
    };
    if body.text.trim().is_empty()
        || body.text.chars().count() > steering::MAX_CHARS
        || body.client_request_id.is_empty()
        || body.client_request_id.len() > 256
        || !body.client_request_id.bytes().all(|b| b.is_ascii_graphic())
        || Uuid::parse_str(&body.turn_id).is_err()
    {
        return Ok(refusal("invalid_request"));
    }
    // Do not provision, replace or rotate a key while steering an existing turn.
    let Some(credential) =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, &owner, &id).await?
    else {
        return Ok(refusal("steer_unavailable"));
    };
    let cap = upstream_capabilities(&state, &auth, &credential, policy).await;
    let Some(cap) = steer_capability(&cap) else {
        return Err(AppError::AssistantTurnActive);
    };
    if body.text.chars().count() > cap.max_chars {
        return Ok(refusal("invalid_request"));
    }
    // Reserve rechecks Stop/liveness in the same transaction that appends the row.
    let message = match Box::pin(steering::reserve(
        &state.db,
        &owner,
        &id,
        &body.turn_id,
        &body.client_request_id,
        &body.text,
    ))
    .await?
    {
        Admission::Refused(code) => return Ok(refusal(code)),
        Admission::Replay(message) => {
            return Ok(receipt_response(
                &steering::replay(&state.db, message).await?,
            ));
        }
        Admission::Dispatch(message) => message,
    };
    let receipt = message.steering.as_ref().unwrap();
    let current = engine::get(&state.db, &owner, &id).await?;
    let local_refusal = steering::unavailable(&current, &body.turn_id).or_else(|| {
        (current.credential_api_key_id != credential.api_key_id
            || row.credential_api_key_id != credential.api_key_id)
            .then_some("no_active_turn")
    });
    let outcome = if let Some(code) = local_refusal {
        ("refused", Some(code), refusal(code).status().as_u16())
    } else {
        let call = async {
            let response = Box::pin(proxy(
                &state,
                &auth,
                &credential,
                "POST",
                &format!("v1/conversations/{}/steer", receipt.session_id),
                Some(json!({"input":body.text,"expected_response_id":receipt.response_id})),
                Some(&body.client_request_id),
                policy,
            ))
            .await
            .ok()?;
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), 16384)
                .await
                .ok()?;
            let value: Value = serde_json::from_slice(&bytes).ok()?;
            if status == 200
                && value["status"] == "accepted"
                && value["object"] == "conversation.steer"
                && value["response_id"] == receipt.response_id
                && value["conversation_id"] == receipt.session_id
            {
                return Some(("applied", None, 200));
            }
            let code = match (status, value["error"]["code"].as_str()) {
                (409, Some("no_active_turn")) => "no_active_turn",
                (409, Some("response_mismatch")) => "response_mismatch",
                (409, Some("idempotency_conflict")) => "idempotency_conflict",
                (429, Some("steer_limit")) => "steer_limit",
                (400, _) => "invalid_request",
                (404, _) => "not_found",
                _ => return None,
            };
            Some(("refused", Some(code), status))
        };
        tokio::time::timeout(Duration::from_secs(15), call)
            .await
            .ok()
            .flatten()
            .unwrap_or(("may_not_have_applied", Some("steer_unavailable"), 503))
    };
    steering::settle(&state.db, &message, outcome.0, outcome.1, outcome.2).await?;
    let mut message = message;
    let receipt = message.steering.as_mut().unwrap();
    receipt.outcome = outcome.0.into();
    receipt.code = outcome.1.map(str::to_owned);
    receipt.http_status = outcome.2;
    Ok(receipt_response(&message))
}
