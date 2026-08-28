use std::time::Instant;

use axum::{
    Extension, Json,
    body::Body,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::service_billing::{BillingMetric, PlatformUsage};
use crate::models::usage_meter::CredentialClass;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::billing::route_inventory::{
    BillingEgressPermit, BillingIngress, BillingRoutePolicy,
};
use crate::services::platform_operation_service::{
    CallAndSayRequest, FlightSearchRequest, PlatformCredentialResolution, PlatformCredentialSource,
    SpeakRequest,
};
use crate::services::{audit_service, feature_flag_service, platform_operation_service};

pub const CREDENTIAL_SOURCE_HEADER: &str = "x-nyxid-credential-source";

#[derive(Clone)]
pub(crate) struct PlatformOperationCaller {
    pub actor_user_id: String,
    pub resolution_user_id: String,
    pub api_key_id: Option<String>,
    pub auth_method: AuthMethod,
    pub acting_client_id: Option<String>,
    pub allow_all_services: bool,
    pub allowed_service_ids: Vec<String>,
}

impl PlatformOperationCaller {
    fn from_auth_user(auth_user: &AuthUser) -> Self {
        Self {
            actor_user_id: auth_user.user_id.to_string(),
            resolution_user_id: auth_user.proxy_resolution_user_id(),
            api_key_id: auth_user.api_key_id.clone(),
            auth_method: auth_user.auth_method.clone(),
            acting_client_id: auth_user.acting_client_id.clone(),
            allow_all_services: auth_user.allow_all_services,
            allowed_service_ids: auth_user.allowed_service_ids.clone(),
        }
    }

    pub(crate) fn credential_caller(
        &self,
    ) -> platform_operation_service::PlatformCredentialCaller<'_> {
        platform_operation_service::PlatformCredentialCaller {
            actor_user_id: &self.actor_user_id,
            api_key_id: self.api_key_id.as_deref(),
            allow_all_services: self.allow_all_services,
            allowed_service_ids: &self.allowed_service_ids,
            bypass_approval_flow: self.auth_method == AuthMethod::Session,
        }
    }

    fn approval_requester_type(&self) -> Option<&'static str> {
        match self.auth_method {
            AuthMethod::ApiKey => Some("api_key"),
            AuthMethod::Delegated => Some("delegated"),
            AuthMethod::ServiceAccount => Some("service_account"),
            AuthMethod::AccessToken => Some("access_token"),
            AuthMethod::Relay => Some("relay"),
            AuthMethod::Session => None,
        }
    }

    fn approval_requester_id(&self) -> &str {
        self.acting_client_id
            .as_deref()
            .unwrap_or(&self.actor_user_id)
    }
}

pub(crate) struct PlatformOperationExecution<T> {
    pub value: T,
    pub credential_source: PlatformCredentialSource,
    pub own_connection_disabled: bool,
    pub own_connection_out_of_scope: bool,
}

enum ExecutionTarget {
    Platform(Box<crate::models::downstream_service::DownstreamService>),
    OwnConnection(Box<OwnConnectionExecutionTarget>),
}

struct OwnConnectionExecutionTarget {
    target: crate::services::proxy_service::ProxyTarget,
    user_service_id: String,
    service_owner_id: String,
    is_auto_connected: bool,
}

struct ResolvedExecutionTarget {
    target: ExecutionTarget,
    credential_source: PlatformCredentialSource,
    own_connection_disabled: bool,
    own_connection_out_of_scope: bool,
}

async fn require_platform_ops_enabled(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let enabled =
        feature_flag_service::resolve_personal_features(&state.db, &auth_user.user_id.to_string())
            .await?
            .into_iter()
            .any(|key| key == feature_flag_service::PLATFORM_SERVICES_FLAG_KEY);
    if !enabled {
        return Err(AppError::NotFound(
            "Platform operation route not found.".to_string(),
        ));
    }
    Ok(())
}

/// Structural feature gate for every route in the `/platform-ops` nest.
/// Handler-level checks remain as belt-and-braces protection for direct calls
/// and tests, but a newly mounted route inherits this middleware automatically.
pub async fn platform_services_feature_gate(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    Ok(next.run(request).await)
}

pub async fn speak(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Extension(billing_route_policy): Extension<BillingRoutePolicy>,
    Json(request): Json<SpeakRequest>,
) -> AppResult<Response> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    ensure_platform_operation_caller(&state, &auth_user).await?;
    enforce_agent_rate_limit(&state, &auth_user)?;

    let started = Instant::now();
    let text_chars = request.text.chars().count();
    let voice_id = request.voice_id.clone();
    let billing_egress_permit = enforce_platform_billing_classification(billing_route_policy)?;
    let result = execute_speak_for_caller(
        &state,
        &PlatformOperationCaller::from_auth_user(&auth_user),
        request,
        BillingIngress::PlatformOperation,
        billing_egress_permit,
    )
    .await;
    audit_operation(
        &state,
        &auth_user,
        "speak",
        &result,
        started,
        json!({
            "text_chars": text_chars,
            "voice_id": voice_id,
        }),
    );

    let result = result?;
    let vendor = result.value;
    let content_length = vendor.response.content_length();
    let stream = vendor.response.bytes_stream();
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/mpeg");
    if let Some(content_length) = content_length {
        builder = builder.header(header::CONTENT_LENGTH, content_length);
    }
    let response = builder
        .body(Body::from_stream(stream))
        .map_err(|error| AppError::Internal(format!("Failed to build speech response: {error}")))?;
    response_with_credential_source(response, result.credential_source)
}

pub async fn call_and_say(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Extension(billing_route_policy): Extension<BillingRoutePolicy>,
    Json(request): Json<CallAndSayRequest>,
) -> AppResult<Response> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    ensure_platform_operation_caller(&state, &auth_user).await?;
    enforce_agent_rate_limit(&state, &auth_user)?;

    let started = Instant::now();
    let message_chars = request.message.chars().count();
    let destination_suffix = if platform_operation_service::is_e164_number(&request.to) {
        platform_operation_service::redacted_destination_suffix(&request.to)
    } else {
        "invalid".to_string()
    };
    // The date is deliberately derived at the transport edge and passed into
    // the service, keeping quota tests deterministic and avoiding hidden time
    // reads in the reservation logic.
    let yyyymmdd = Utc::now().format("%Y%m%d").to_string();
    let billing_egress_permit = enforce_platform_billing_classification(billing_route_policy)?;
    let result = execute_call_and_say_for_caller(
        &state,
        &PlatformOperationCaller::from_auth_user(&auth_user),
        &yyyymmdd,
        request,
        BillingIngress::PlatformOperation,
        billing_egress_permit,
    )
    .await;
    audit_operation(
        &state,
        &auth_user,
        "call_and_say",
        &result,
        started,
        json!({
            "message_chars": message_chars,
            "destination_suffix": destination_suffix,
        }),
    );

    let result = result?;
    response_with_credential_source(Json(result.value).into_response(), result.credential_source)
}

pub async fn flight_search(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Extension(billing_route_policy): Extension<BillingRoutePolicy>,
    Json(request): Json<FlightSearchRequest>,
) -> AppResult<Response> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    ensure_platform_operation_caller(&state, &auth_user).await?;
    enforce_agent_rate_limit(&state, &auth_user)?;

    let started = Instant::now();
    let requested_max_offers = request.max_offers;
    let yyyymmdd = Utc::now().format("%Y%m%d").to_string();
    let billing_egress_permit = enforce_platform_billing_classification(billing_route_policy)?;
    let result = execute_flight_search_for_caller(
        &state,
        &PlatformOperationCaller::from_auth_user(&auth_user),
        &yyyymmdd,
        request,
        BillingIngress::PlatformOperation,
        billing_egress_permit,
    )
    .await;
    audit_operation(
        &state,
        &auth_user,
        "flight_search",
        &result,
        started,
        json!({ "requested_max_offers": requested_max_offers }),
    );

    let result = result?;
    response_with_credential_source(Json(result.value).into_response(), result.credential_source)
}

fn enforce_platform_billing_classification(
    policy: BillingRoutePolicy,
) -> AppResult<BillingEgressPermit> {
    crate::services::billing::route_inventory::enforce_billing_egress_classification(
        Some(policy),
        BillingIngress::PlatformOperation,
    )
}

fn response_with_credential_source(
    mut response: Response,
    source: PlatformCredentialSource,
) -> AppResult<Response> {
    response.headers_mut().insert(
        CREDENTIAL_SOURCE_HEADER,
        header::HeaderValue::from_static(source.as_str()),
    );
    Ok(response)
}

#[derive(Serialize)]
pub struct PlatformOperationsResponse {
    pub(crate) operations: Vec<PlatformOperationDiscoveryResponse>,
}

#[derive(Serialize)]
pub struct PlatformOperationDiscoveryResponse {
    op: String,
    display_name: String,
    description: String,
    vendor: String,
    catalog_service_slug: String,
    credential_source: PlatformCredentialSource,
    own_connection: Option<OwnConnectionDiscoveryResponse>,
    pricing: PlatformOperationPricingResponse,
    mcp_tool: String,
}

#[derive(Serialize)]
pub struct OwnConnectionDiscoveryResponse {
    pub(crate) user_service_id: String,
    pub(crate) slug: String,
    pub(crate) label: String,
    pub(crate) is_active: bool,
    pub(crate) usable: bool,
    pub(crate) reason: Option<&'static str>,
}

#[derive(Serialize)]
pub struct PlatformOperationPricingResponse {
    billable: bool,
    metric: &'static str,
    price_per_unit: String,
    base_fee_per_call: Option<String>,
    /// Server-rendered price sentence, already gated on the billing rollout
    /// flag. A caller with the rollout off sees "Free" because that is what
    /// the call will actually cost them.
    display: String,
}

/// Pure projection from a stored operation to its discovery row.
///
/// Kept separate from `list_operations` so the cross-language contract test can
/// serialize the exact shape the handler returns rather than a hand-written
/// copy of it.
pub(crate) fn platform_operation_discovery_response(
    operation: &crate::models::platform_operation::PlatformOperation,
    credential_source: PlatformCredentialSource,
    own_connection: Option<OwnConnectionDiscoveryResponse>,
    rollout_enabled: bool,
) -> PlatformOperationDiscoveryResponse {
    let contract = platform_operation_service::catalog_contract_for_operation(operation.op);
    let price = operation.billing.price_per_unit.clone();
    let billable =
        rollout_enabled && (price != "0" || operation.billing.base_fee_per_call.is_some());
    PlatformOperationDiscoveryResponse {
        op: platform_operation_service::operation_name(operation.op).to_string(),
        display_name: contract.display_name.to_string(),
        description: contract.description.to_string(),
        vendor: contract.vendor.to_string(),
        catalog_service_slug: contract.catalog_service_slug.to_string(),
        credential_source,
        own_connection,
        pricing: PlatformOperationPricingResponse {
            billable,
            metric: operation.billing.metric.as_str(),
            display: crate::services::billing::pricing::format_operation_price(
                &operation.billing,
                billable,
            ),
            price_per_unit: price,
            base_fee_per_call: operation.billing.base_fee_per_call.clone(),
        },
        mcp_tool: contract.mcp_tool.to_string(),
    }
}

pub async fn list_operations(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<PlatformOperationsResponse>> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    ensure_platform_operation_caller(&state, &auth_user).await?;
    let resolution_user_id = auth_user.proxy_resolution_user_id();
    let rollout_enabled = feature_flag_service::billing_rollout_enabled(
        &state.db,
        &resolution_user_id,
        &auth_user.user_id.to_string(),
    )
    .await?;
    let operations = platform_operation_service::list_enabled_operations(&state.db).await?;
    let context = platform_operation_service::load_credential_resolution_context(
        &state.db,
        &resolution_user_id,
        &operations,
    )
    .await?;
    let caller = PlatformOperationCaller::from_auth_user(&auth_user);
    let mut response = Vec::with_capacity(operations.len());
    for operation in operations {
        let descriptor = platform_operation_discovery_descriptor(operation.op);
        let source = platform_operation_service::resolve_operation_credential_source(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &resolution_user_id,
            caller.credential_caller(),
            &operation,
            platform_operation_service::CredentialResolutionMode::Discover {
                descriptor: &descriptor,
            },
            &context,
        )
        .await?;
        let (credential_source, own_connection) = discovery_source(source);
        response.push(platform_operation_discovery_response(
            &operation,
            credential_source,
            own_connection,
            rollout_enabled,
        ));
    }
    Ok(Json(PlatformOperationsResponse {
        operations: response,
    }))
}

pub(crate) fn platform_tool_credential_sentence(
    operation: &crate::models::platform_operation::PlatformOperation,
    source: &PlatformCredentialResolution,
    _vendor: Option<&crate::models::downstream_service::DownstreamService>,
    rollout_enabled: bool,
) -> String {
    let contract = platform_operation_service::catalog_contract_for_operation(operation.op);
    match source {
        PlatformCredentialResolution::OwnConnection { .. } => {
            return format!(
                "Uses your connected {} account.",
                vendor_display_name(contract.vendor)
            );
        }
        PlatformCredentialResolution::NodeRouted { .. } => {
            return format!(
                "Your {} connection is node-routed; calls fail until you disable it or connect a server-held key.",
                vendor_display_name(contract.vendor)
            );
        }
        PlatformCredentialResolution::ApprovalRequired { .. } => {
            return format!(
                "Your {} connection requires approval per request; calls fail until you approve-free or disable it.",
                vendor_display_name(contract.vendor)
            );
        }
        PlatformCredentialResolution::Unusable { .. } => {
            return format!(
                "Your {} connection is unusable; reconnect or disable it.",
                vendor_display_name(contract.vendor)
            );
        }
        PlatformCredentialResolution::Platform { .. } => {}
    }

    let billing = &operation.billing;
    let mut sentence = if !rollout_enabled
        || (billing.price_per_unit == "0" && billing.base_fee_per_call.is_none())
    {
        "Uses the platform credential (free).".to_string()
    } else {
        // Shared with the admin table and /keys so one operation cannot be
        // described in three different units across three surfaces.
        let unit = billing.metric.unit_noun();
        match billing.base_fee_per_call.as_deref() {
            Some(base) => format!(
                "Uses the platform credential ({base} credits plus {} credits per {unit}).",
                billing.price_per_unit
            ),
            None => format!(
                "Uses the platform credential ({} credits per {unit}).",
                billing.price_per_unit
            ),
        }
    };
    if let crate::models::platform_operation::PlatformOperationConfig::Speak(config) =
        &operation.config
        && !config.allowed_voice_ids.is_empty()
    {
        sentence.push_str(" Allowed voice ids: ");
        sentence.push_str(&config.allowed_voice_ids.join(", "));
        sentence.push('.');
    }
    sentence
}

fn discovery_source(
    source: PlatformCredentialResolution,
) -> (
    PlatformCredentialSource,
    Option<OwnConnectionDiscoveryResponse>,
) {
    match source {
        PlatformCredentialResolution::Platform {
            disabled_connection,
            ..
        } => (
            PlatformCredentialSource::Platform,
            disabled_connection
                .map(|connection| own_connection_response(connection, false, Some("disabled"))),
        ),
        PlatformCredentialResolution::OwnConnection { connection, .. } => (
            PlatformCredentialSource::OwnConnection,
            Some(own_connection_response(connection, true, None)),
        ),
        PlatformCredentialResolution::NodeRouted { connection } => (
            PlatformCredentialSource::OwnConnection,
            Some(own_connection_response(
                connection,
                false,
                Some("node_routed"),
            )),
        ),
        PlatformCredentialResolution::ApprovalRequired { connection } => (
            PlatformCredentialSource::OwnConnection,
            Some(own_connection_response(
                connection,
                false,
                Some("approval_required"),
            )),
        ),
        PlatformCredentialResolution::Unusable { connection, .. } => (
            PlatformCredentialSource::OwnConnection,
            Some(own_connection_response(connection, false, Some("unusable"))),
        ),
    }
}

pub(crate) fn platform_operation_discovery_descriptor(
    op: crate::models::platform_operation::PlatformOperationName,
) -> crate::services::operation_descriptor::OperationDescriptor {
    use crate::models::platform_operation::PlatformOperationName;

    let (method, path) = match op {
        PlatformOperationName::Speak => ("POST", "/v1/text-to-speech/{voice_id}"),
        PlatformOperationName::CallAndSay => {
            ("POST", "/2010-04-01/Accounts/{account_sid}/Calls.json")
        }
        PlatformOperationName::FlightSearch => ("POST", "/air/offer_requests"),
    };
    crate::services::operation_descriptor::build_http_descriptor(method, path, None)
}

fn own_connection_response(
    connection: platform_operation_service::OwnConnectionMetadata,
    usable: bool,
    reason: Option<&'static str>,
) -> OwnConnectionDiscoveryResponse {
    OwnConnectionDiscoveryResponse {
        user_service_id: connection.user_service_id,
        slug: connection.slug,
        label: connection.label,
        is_active: connection.is_active,
        usable,
        reason,
    }
}

struct DailyLimit {
    yyyymmdd: String,
    cap: u32,
}

struct ForwardedOperation {
    response: reqwest::Response,
    credential_source: PlatformCredentialSource,
    own_connection_disabled: bool,
    own_connection_out_of_scope: bool,
    metered: crate::services::billing::MeteredProxyContext,
    daily_limit: Option<DailyLimit>,
}

async fn resolve_execution_target(
    state: &AppState,
    caller: &PlatformOperationCaller,
    operation: &crate::models::platform_operation::PlatformOperation,
) -> AppResult<ResolvedExecutionTarget> {
    let contract = platform_operation_service::catalog_contract_for_operation(operation.op);
    let context = platform_operation_service::load_credential_resolution_context(
        &state.db,
        &caller.resolution_user_id,
        std::slice::from_ref(operation),
    )
    .await?;
    match platform_operation_service::resolve_operation_credential_source(
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &caller.resolution_user_id,
        caller.credential_caller(),
        operation,
        platform_operation_service::CredentialResolutionMode::Execute {
            connection_expiry_notifier: Some(&state.connection_expiry_notifier),
        },
        &context,
    )
    .await?
    {
        PlatformCredentialResolution::Platform {
            vendor,
            disabled_connection,
            own_connection_out_of_scope,
        } => Ok(ResolvedExecutionTarget {
            target: ExecutionTarget::Platform(vendor),
            credential_source: PlatformCredentialSource::Platform,
            own_connection_disabled: disabled_connection.is_some(),
            own_connection_out_of_scope,
        }),
        PlatformCredentialResolution::OwnConnection { resolution, .. } => {
            let resolution = *resolution;
            let service_owner_id = resolution
                .org_routing
                .as_ref()
                .map(|routing| routing.org_user_id.clone())
                .unwrap_or_else(|| caller.resolution_user_id.clone());
            Ok(ResolvedExecutionTarget {
                target: ExecutionTarget::OwnConnection(Box::new(OwnConnectionExecutionTarget {
                    target: resolution.target,
                    user_service_id: resolution.user_service_id,
                    service_owner_id,
                    is_auto_connected: resolution.is_auto_connected,
                })),
                credential_source: PlatformCredentialSource::OwnConnection,
                own_connection_disabled: false,
                own_connection_out_of_scope: false,
            })
        }
        PlatformCredentialResolution::NodeRouted { .. } => {
            Err(AppError::PlatformOperationOwnConnectionUnsupported {
                vendor: vendor_display_name(contract.vendor),
            })
        }
        PlatformCredentialResolution::ApprovalRequired { .. } => {
            Err(AppError::PlatformOperationApprovalRequired {
                vendor: vendor_display_name(contract.vendor),
            })
        }
        PlatformCredentialResolution::Unusable { error, .. } => {
            Err(
                error.unwrap_or_else(|| AppError::PlatformOperationOwnConnectionUnusable {
                    vendor: vendor_display_name(contract.vendor),
                    detail: "Reconnect or disable it before retrying.".to_string(),
                }),
            )
        }
    }
}

fn vendor_display_name(vendor: &str) -> String {
    platform_operation_service::vendor_display_name(vendor)
}

#[allow(clippy::too_many_arguments)]
async fn forward_metered_operation(
    state: &AppState,
    caller: &PlatformOperationCaller,
    ingress: BillingIngress,
    operation: &crate::models::platform_operation::PlatformOperation,
    platform_estimated_quantity: i64,
    resolved: ResolvedExecutionTarget,
    method: reqwest::Method,
    path: &str,
    query: Option<&str>,
    headers: reqwest::header::HeaderMap,
    body: Option<bytes::Bytes>,
    platform_basic_username: Option<&str>,
    daily_limit: Option<DailyLimit>,
    billing_egress_permit: BillingEgressPermit,
) -> AppResult<ForwardedOperation> {
    let op = operation.op;
    if let ExecutionTarget::OwnConnection(own) = &resolved.target {
        let descriptor = crate::services::operation_descriptor::build_http_descriptor(
            method.as_str(),
            path,
            body.as_deref(),
        );
        let approval = crate::services::approval_service::evaluate_and_check(
            &state.db,
            &caller.actor_user_id,
            &own.service_owner_id,
            &own.user_service_id,
            &descriptor,
            caller.approval_requester_type(),
            caller.approval_requester_id(),
            caller.auth_method == AuthMethod::Session,
            own.is_auto_connected,
        )
        .await?;
        match approval {
            crate::services::approval_service::ApprovalOutcome::Allowed { .. } => {}
            crate::services::approval_service::ApprovalOutcome::Denied => {
                return Err(AppError::Forbidden(
                    "Operation denied by approval policy".to_string(),
                ));
            }
            crate::services::approval_service::ApprovalOutcome::NeedsApproval(_) => {
                let contract = platform_operation_service::catalog_contract_for_operation(op);
                return Err(AppError::PlatformOperationApprovalRequired {
                    vendor: vendor_display_name(contract.vendor),
                });
            }
        }
    }

    let is_platform = matches!(&resolved.target, ExecutionTarget::Platform(_));
    if is_platform && let Some(limit) = &daily_limit {
        platform_operation_service::reserve_daily_operation(
            &state.db,
            op,
            &caller.resolution_user_id,
            &limit.yyyymmdd,
            limit.cap,
        )
        .await?;
    }

    let metered = if let ExecutionTarget::Platform(vendor) = &resolved.target {
        let billing_owner = state
            .billing
            .owner_resolver()
            .resolve_for_resource(&caller.resolution_user_id, &caller.resolution_user_id)
            .await;
        let billing_owner = match billing_owner {
            Ok(owner) => owner,
            Err(error) => {
                release_daily_limit(state, op, caller, daily_limit.as_ref()).await;
                return Err(error);
            }
        };
        let base_fee_micros = match operation.billing.base_fee_per_call.as_deref() {
            Some(base_fee) => {
                crate::services::billing::lago_client::decimal_credits_to_micros(base_fee)
                    .ok_or(AppError::PlatformOperationUnavailable)?
            }
            None => 0,
        };
        let billing_ctx = crate::services::billing::BillingRouteContext::new(
            ingress,
            uuid::Uuid::new_v4().to_string(),
            billing_owner.owner_id,
            caller.actor_user_id.clone(),
            caller.api_key_id.clone(),
            None,
            Some(vendor.id.clone()),
            Some(vendor.slug.clone()),
            crate::services::billing::NodeIntent::Direct,
            vendor.auth_method.clone(),
            CredentialClass::NyxidManagedMaster,
            BillingMetric::Requests,
            None,
            false,
        )
        .with_platform_operation_billing(
            operation.billing.metric,
            operation.billing.lago_metric_code.clone(),
            crate::services::billing::lago_client::decimal_credits_to_micros(
                &operation.billing.price_per_unit,
            )
            .ok_or_else(|| AppError::PlatformOperationUnavailable)?,
            base_fee_micros,
            platform_estimated_quantity,
        );
        match state.billing.open(&billing_ctx).await {
            Ok(metered) => metered,
            Err(error) => {
                release_daily_limit(state, op, caller, daily_limit.as_ref()).await;
                return Err(error);
            }
        }
    } else {
        crate::services::billing::MeteredProxyContext::disabled()
    };

    let target = match resolved.target {
        ExecutionTarget::Platform(vendor) => {
            match platform_operation_service::materialize_platform_vendor_target(
                &state.db,
                &state.encryption_keys,
                op,
                *vendor,
            )
            .await
            {
                Ok(mut target) => {
                    if let Some(username) = platform_basic_username {
                        target.target.credential =
                            format!("{username}:{}", target.target.credential);
                    }
                    target.target
                }
                Err(error) => {
                    fail_platform_attempt(
                        state,
                        &metered,
                        "credential_unavailable",
                        op,
                        caller,
                        daily_limit.as_ref(),
                    )
                    .await;
                    return Err(error);
                }
            }
        }
        ExecutionTarget::OwnConnection(own) => own.target,
    };

    if let Err(error) = state.billing.mark_forwarded(&metered).await {
        fail_platform_attempt(
            state,
            &metered,
            "mark_forwarded_failed",
            op,
            caller,
            daily_limit.as_ref(),
        )
        .await;
        return Err(error);
    }
    let response = platform_operation_service::forward_operation_request(
        &state.http_client,
        &target,
        op,
        method,
        path,
        query,
        headers,
        body,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        billing_egress_permit,
    )
    .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            fail_platform_attempt(
                state,
                &metered,
                "vendor_request_failed",
                op,
                caller,
                daily_limit.as_ref(),
            )
            .await;
            return Err(error);
        }
    };
    if let Err(error) = platform_operation_service::ensure_vendor_success(op, &response) {
        fail_platform_attempt(
            state,
            &metered,
            "vendor_non_success",
            op,
            caller,
            daily_limit.as_ref(),
        )
        .await;
        return Err(error);
    }
    Ok(ForwardedOperation {
        response,
        credential_source: resolved.credential_source,
        own_connection_disabled: resolved.own_connection_disabled,
        own_connection_out_of_scope: resolved.own_connection_out_of_scope,
        metered,
        daily_limit,
    })
}

async fn settle_meter_async(
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    metered: crate::services::billing::MeteredProxyContext,
    usage: PlatformUsage,
) {
    if !metered.is_enabled() {
        return;
    }
    if billing
        .settle_deferred(&metered, usage, None, None)
        .await
        .is_err()
    {
        let billing_request_id = metered
            .route
            .as_ref()
            .map(|route| route.billing_request_id.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            billing_request_id,
            "Failed to persist platform operation settlement intent"
        );
    }
}

async fn fail_platform_attempt(
    state: &AppState,
    metered: &crate::services::billing::MeteredProxyContext,
    reason: &str,
    op: crate::models::platform_operation::PlatformOperationName,
    caller: &PlatformOperationCaller,
    daily_limit: Option<&DailyLimit>,
) {
    if let Err(error) = state.billing.fail(metered, reason).await {
        tracing::error!(
            op = platform_operation_service::operation_name(op),
            error = %error,
            "Failed to release platform operation billing reservation"
        );
    }
    release_daily_limit(state, op, caller, daily_limit).await;
}

async fn release_daily_limit(
    state: &AppState,
    op: crate::models::platform_operation::PlatformOperationName,
    caller: &PlatformOperationCaller,
    daily_limit: Option<&DailyLimit>,
) {
    let Some(limit) = daily_limit else {
        return;
    };
    if let Err(error) = platform_operation_service::release_daily_operation(
        &state.db,
        op,
        &caller.resolution_user_id,
        &limit.yyyymmdd,
    )
    .await
    {
        tracing::error!(
            op = platform_operation_service::operation_name(op),
            error = %error,
            "Failed to release platform operation daily reservation"
        );
    }
}

pub(crate) async fn execute_speak_for_caller(
    state: &AppState,
    caller: &PlatformOperationCaller,
    request: SpeakRequest,
    ingress: BillingIngress,
    billing_egress_permit: BillingEgressPermit,
) -> AppResult<PlatformOperationExecution<platform_operation_service::SpeakVendorResponse>> {
    use crate::models::platform_operation::{PlatformOperationConfig, PlatformOperationName};

    let operation =
        platform_operation_service::load_enabled_operation(&state.db, PlatformOperationName::Speak)
            .await?;
    let PlatformOperationConfig::Speak(config) = &operation.config else {
        return Err(AppError::PlatformOperationUnavailable);
    };
    let text_characters = request.text.chars().count() as i64;
    let resolved = resolve_execution_target(state, caller, &operation).await?;
    let enforce_allowlist = resolved.credential_source == PlatformCredentialSource::Platform;
    let upstream = platform_operation_service::build_speak_request_for_source(
        config,
        &request,
        enforce_allowlist,
    )?;
    let result = forward_metered_operation(
        state,
        caller,
        ingress,
        &operation,
        text_characters,
        resolved,
        reqwest::Method::POST,
        &upstream.path,
        None,
        platform_operation_service::json_request_headers(),
        Some(bytes::Bytes::from(
            serde_json::to_vec(&upstream.body).map_err(|error| {
                AppError::Internal(format!("Failed to encode speak request: {error}"))
            })?,
        )),
        None,
        None,
        billing_egress_permit,
    )
    .await?;
    let ForwardedOperation {
        response,
        credential_source,
        own_connection_disabled,
        own_connection_out_of_scope,
        metered,
        ..
    } = result;
    settle_meter_async(
        state.billing.clone(),
        metered,
        PlatformUsage::single_request(0).with_characters(text_characters),
    )
    .await;
    Ok(PlatformOperationExecution {
        value: platform_operation_service::SpeakVendorResponse { response },
        credential_source,
        own_connection_disabled,
        own_connection_out_of_scope,
    })
}

pub(crate) async fn execute_call_and_say_for_caller(
    state: &AppState,
    caller: &PlatformOperationCaller,
    yyyymmdd: &str,
    request: CallAndSayRequest,
    ingress: BillingIngress,
    billing_egress_permit: BillingEgressPermit,
) -> AppResult<PlatformOperationExecution<serde_json::Value>> {
    use crate::models::platform_operation::{PlatformOperationConfig, PlatformOperationName};

    let operation = platform_operation_service::load_enabled_operation(
        &state.db,
        PlatformOperationName::CallAndSay,
    )
    .await?;
    let PlatformOperationConfig::CallAndSay(config) = &operation.config else {
        return Err(AppError::PlatformOperationUnavailable);
    };
    let resolved = resolve_execution_target(state, caller, &operation).await?;
    let identity = match &resolved.target {
        ExecutionTarget::Platform(_) => {
            platform_operation_service::CallCredentialIdentity::Platform
        }
        ExecutionTarget::OwnConnection(own) => {
            platform_operation_service::CallCredentialIdentity::OwnConnection {
                credential: &own.target.credential,
            }
        }
    };
    let upstream = platform_operation_service::build_call_and_say_request_for_source(
        config, &request, identity,
    )?;
    let form = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            upstream
                .form
                .iter()
                .map(|(name, value)| (*name, value.as_str())),
        )
        .finish();
    let daily_limit =
        (resolved.credential_source == PlatformCredentialSource::Platform).then_some(DailyLimit {
            yyyymmdd: yyyymmdd.to_string(),
            cap: config.max_calls_per_user_per_day,
        });
    let result = forward_metered_operation(
        state,
        caller,
        ingress,
        &operation,
        i64::from(
            config
                .max_duration_seconds
                .min(platform_operation_service::CALL_AND_SAY_HARD_MAX_DURATION_SECONDS),
        ),
        resolved,
        reqwest::Method::POST,
        &upstream.path,
        None,
        platform_operation_service::form_request_headers(),
        Some(bytes::Bytes::from(form)),
        Some(&config.account_sid),
        daily_limit,
        billing_egress_permit,
    )
    .await?;
    let ForwardedOperation {
        response,
        credential_source,
        own_connection_disabled,
        own_connection_out_of_scope,
        metered,
        daily_limit,
    } = result;
    let value =
        platform_operation_service::read_vendor_json(PlatformOperationName::CallAndSay, response)
            .await;
    let value = match value {
        Ok(value) => value,
        Err(error) => {
            fail_platform_attempt(
                state,
                &metered,
                "vendor_response_invalid",
                PlatformOperationName::CallAndSay,
                caller,
                daily_limit.as_ref(),
            )
            .await;
            return Err(error);
        }
    };
    if credential_source == PlatformCredentialSource::Platform {
        let call_sid = value
            .get("sid")
            .and_then(serde_json::Value::as_str)
            .filter(|sid| platform_operation_service::is_twilio_call_sid(sid));
        let Some(call_sid) = call_sid else {
            fail_platform_attempt(
                state,
                &metered,
                "vendor_call_sid_invalid",
                PlatformOperationName::CallAndSay,
                caller,
                daily_limit.as_ref(),
            )
            .await;
            return Err(AppError::PlatformOperationUnavailable);
        };
        state
            .billing
            .defer_twilio_call(&metered, config.account_sid.clone(), call_sid.to_string())
            .await?;
    } else {
        settle_meter_async(
            state.billing.clone(),
            metered,
            PlatformUsage::single_request(0).with_seconds(0),
        )
        .await;
    }
    Ok(PlatformOperationExecution {
        value,
        credential_source,
        own_connection_disabled,
        own_connection_out_of_scope,
    })
}

pub(crate) async fn execute_flight_search_for_caller(
    state: &AppState,
    caller: &PlatformOperationCaller,
    yyyymmdd: &str,
    request: FlightSearchRequest,
    ingress: BillingIngress,
    billing_egress_permit: BillingEgressPermit,
) -> AppResult<PlatformOperationExecution<platform_operation_service::FlightSearchResponse>> {
    use crate::models::platform_operation::{PlatformOperationConfig, PlatformOperationName};

    let operation = platform_operation_service::load_enabled_operation(
        &state.db,
        PlatformOperationName::FlightSearch,
    )
    .await?;
    let PlatformOperationConfig::FlightSearch(config) = &operation.config else {
        return Err(AppError::PlatformOperationUnavailable);
    };
    let upstream = platform_operation_service::build_flight_search_request(config, &request)?;
    let resolved = resolve_execution_target(state, caller, &operation).await?;
    let daily_limit =
        (resolved.credential_source == PlatformCredentialSource::Platform).then_some(DailyLimit {
            yyyymmdd: yyyymmdd.to_string(),
            cap: config.max_searches_per_user_per_day,
        });
    let result = forward_metered_operation(
        state,
        caller,
        ingress,
        &operation,
        1,
        resolved,
        reqwest::Method::POST,
        upstream.path,
        Some(upstream.query),
        platform_operation_service::duffel_request_headers(),
        Some(bytes::Bytes::from(
            serde_json::to_vec(&upstream.body).map_err(|error| {
                AppError::Internal(format!("Failed to encode flight search request: {error}"))
            })?,
        )),
        None,
        daily_limit,
        billing_egress_permit,
    )
    .await?;
    let ForwardedOperation {
        response,
        credential_source,
        own_connection_disabled,
        own_connection_out_of_scope,
        metered,
        daily_limit,
    } = result;
    let value = async {
        let value = platform_operation_service::read_vendor_json(
            PlatformOperationName::FlightSearch,
            response,
        )
        .await?;
        platform_operation_service::project_flight_search_response(value, upstream.max_offers)
    }
    .await;
    let value = match value {
        Ok(value) => value,
        Err(error) => {
            fail_platform_attempt(
                state,
                &metered,
                "vendor_response_invalid",
                PlatformOperationName::FlightSearch,
                caller,
                daily_limit.as_ref(),
            )
            .await;
            return Err(error);
        }
    };
    settle_meter_async(
        state.billing.clone(),
        metered,
        PlatformUsage::single_request(0),
    )
    .await;
    Ok(PlatformOperationExecution {
        value,
        credential_source,
        own_connection_disabled,
        own_connection_out_of_scope,
    })
}

async fn ensure_platform_operation_caller(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    match auth_user.auth_method {
        AuthMethod::Session | AuthMethod::AccessToken => Ok(()),
        AuthMethod::ApiKey => {
            let api_key_id = auth_user.api_key_id.as_deref().ok_or_else(|| {
                AppError::Unauthorized("Agent API key identity is missing".to_string())
            })?;
            let is_agent_key = state
                .db
                .collection::<ApiKey>(API_KEYS)
                .find_one(mongodb::bson::doc! {
                    "_id": api_key_id,
                    "key_prefix": { "$regex": r"^nyxid_ag_" },
                    "is_active": true,
                })
                .await?
                .is_some();
            if is_agent_key {
                Ok(())
            } else {
                Err(AppError::Forbidden(
                    "A nyxid_ag_ agent API key is required for API-key access.".to_string(),
                ))
            }
        }
        AuthMethod::Delegated | AuthMethod::Relay | AuthMethod::ServiceAccount => Err(
            AppError::Forbidden("This token type cannot access platform operations.".to_string()),
        ),
    }
}

fn enforce_agent_rate_limit(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    crate::mw::rate_limit::check_agent_rate_limit_raw(
        &state.per_agent_limiter,
        auth_user.api_key_id.as_deref(),
        auth_user.rate_limit_per_second,
        auth_user.rate_limit_burst,
    )
}

fn audit_operation<T>(
    state: &AppState,
    auth_user: &AuthUser,
    op: &'static str,
    result: &AppResult<PlatformOperationExecution<T>>,
    started: Instant,
    metadata: Value,
) {
    let data = platform_operation_audit_metadata(op, result, started, metadata);
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "platform_operation",
        Some(Value::Object(data)),
    );
}

pub(crate) fn platform_operation_audit_metadata<T>(
    op: &'static str,
    result: &AppResult<PlatformOperationExecution<T>>,
    started: Instant,
    metadata: Value,
) -> Map<String, Value> {
    let mut data = match metadata {
        Value::Object(data) => data,
        _ => Map::new(),
    };
    data.insert("op".to_string(), Value::String(op.to_string()));
    data.insert(
        "outcome".to_string(),
        Value::String(audit_outcome(result).to_string()),
    );
    data.insert(
        "duration_ms".to_string(),
        Value::from(started.elapsed().as_millis() as u64),
    );
    if let Ok(execution) = result {
        data.insert(
            "credential_source".to_string(),
            Value::String(execution.credential_source.as_str().to_string()),
        );
        if execution.own_connection_disabled {
            data.insert("own_connection_disabled".to_string(), Value::Bool(true));
        }
        if execution.own_connection_out_of_scope {
            data.insert("own_connection_out_of_scope".to_string(), Value::Bool(true));
        }
    }
    data
}

fn audit_outcome<T>(result: &AppResult<T>) -> &'static str {
    match result {
        Ok(_) => "succeeded",
        Err(AppError::TokenExpired) => "own_connection_unusable",
        Err(AppError::NotFound(_)) => "not_found",
        Err(AppError::RateLimited) => "rate_limited",
        Err(AppError::InsufficientCredits) => "insufficient_credits",
        Err(
            AppError::PlatformOperationOwnConnectionUnsupported { .. }
            | AppError::PlatformOperationOwnConnectionUnusable { .. },
        ) => "own_connection_unusable",
        Err(AppError::PlatformOperationApprovalRequired { .. }) => "approval_required",
        Err(AppError::BadRequest(_) | AppError::ValidationError(_)) => "rejected",
        Err(AppError::PlatformOperationUnavailable) => "vendor_unavailable",
        Err(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        http::{HeaderMap, Method, Uri},
        routing::{any, post},
    };
    use base64::Engine;
    use mongodb::{IndexModel, options::IndexOptions};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;

    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::platform_op_usage::{COLLECTION_NAME as PLATFORM_OP_USAGE, PlatformOpUsage};
    use crate::models::platform_operation::{
        COLLECTION_NAME as PLATFORM_OPERATIONS, CallAndSayConfig, CallAndSayOperationConfig,
        ConstrainedConfig, FlightSearchOperationConfig, OperationBilling, OperationLimits,
        PerRequestCaps, PlatformOperation, PlatformOperationConfig, PlatformOperationKind,
        PlatformOperationName, PlatformOperationRow, SpeakConfig, SpeakOperationConfig,
    };
    use crate::models::service_billing::BillingMetric;
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

    const USER_ID: &str = "65de27dc-8cf8-44b6-b8d2-5304e4a90aa4";

    fn operation(
        op: PlatformOperationName,
        enabled: bool,
        config: PlatformOperationConfig,
    ) -> PlatformOperationRow {
        let (kind, limits) = match config {
            PlatformOperationConfig::Speak(config) => (
                PlatformOperationKind::Constrained {
                    op,
                    config: ConstrainedConfig::Speak(SpeakOperationConfig {
                        allowed_voice_ids: config.allowed_voice_ids,
                        model_id: config.model_id,
                    }),
                },
                OperationLimits {
                    per_request: PerRequestCaps::Speak {
                        max_chars: config.max_chars,
                    },
                    per_user_per_day: None,
                },
            ),
            PlatformOperationConfig::CallAndSay(config) => (
                PlatformOperationKind::Constrained {
                    op,
                    config: ConstrainedConfig::CallAndSay(CallAndSayOperationConfig {
                        allowed_destination_prefixes: config.allowed_destination_prefixes,
                        voice: config.voice,
                        account_sid: config.account_sid,
                        call_from: config.call_from,
                    }),
                },
                OperationLimits {
                    per_request: PerRequestCaps::CallAndSay {
                        max_message_chars: config.max_message_chars,
                        max_duration_seconds: config.max_duration_seconds,
                    },
                    per_user_per_day: Some(config.max_calls_per_user_per_day),
                },
            ),
            PlatformOperationConfig::FlightSearch(config) => (
                PlatformOperationKind::Constrained {
                    op,
                    config: ConstrainedConfig::FlightSearch(FlightSearchOperationConfig::default()),
                },
                OperationLimits {
                    per_request: PerRequestCaps::FlightSearch {
                        max_offers: config.max_offers_cap,
                    },
                    per_user_per_day: Some(config.max_searches_per_user_per_day),
                },
            ),
        };
        let mut row = PlatformOperationRow::new_constrained(
            catalog_service_id(platform_operation_service::default_vendor_service_slug(op)),
            op,
            match kind {
                PlatformOperationKind::Constrained { config, .. } => config,
                PlatformOperationKind::Endpoint { .. } => unreachable!(),
            },
            limits,
            OperationBilling::free(BillingMetric::Requests),
            USER_ID.to_string(),
        );
        row.enabled = enabled;
        row
    }

    fn catalog_service_id(slug: &str) -> String {
        format!("test-catalog-{slug}")
    }

    fn call_config(cap: u32) -> CallAndSayConfig {
        CallAndSayConfig {
            allowed_destination_prefixes: vec!["+65".to_string()],
            max_message_chars: 500,
            max_duration_seconds: 600,
            voice: "alice".to_string(),
            max_calls_per_user_per_day: cap,
            account_sid: format!("AC{}", "1".repeat(32)),
            call_from: "+16505550100".to_string(),
        }
    }

    fn billing_extension() -> Extension<BillingRoutePolicy> {
        Extension(BillingRoutePolicy::Metered(
            BillingIngress::PlatformOperation,
        ))
    }

    async fn insert_twilio_vendor(state: &AppState, base_url: String) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = catalog_service_id(platform_operation_service::CALL_AND_SAY_VENDOR_SLUG);
        service.slug = platform_operation_service::CALL_AND_SAY_VENDOR_SLUG.to_string();
        service.name = "Platform Twilio".to_string();
        service.base_url = base_url;
        service.service_category = "internal".to_string();
        service.visibility = "public".to_string();
        service.auth_method = "basic".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.credential_encrypted = Vec::new();
        state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert Twilio vendor row");
        crate::services::platform_credential_service::set_credential(
            &state.db,
            &state.encryption_keys,
            &service.id,
            "twilio-auth-token",
            USER_ID,
        )
        .await
        .expect("set Twilio platform credential");
        service
    }

    async fn insert_speak_vendor(
        state: &AppState,
        base_url: String,
        credential: &str,
    ) -> DownstreamService {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = catalog_service_id(platform_operation_service::SPEAK_VENDOR_SLUG);
        service.slug = platform_operation_service::SPEAK_VENDOR_SLUG.to_string();
        service.name = "Platform ElevenLabs".to_string();
        service.base_url = base_url;
        service.service_category = "internal".to_string();
        service.visibility = "public".to_string();
        service.auth_method = "header".to_string();
        service.auth_key_name = "xi-api-key".to_string();
        service.requires_user_credential = false;
        service.credential_encrypted = Vec::new();
        state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert ElevenLabs vendor row");
        crate::services::platform_credential_service::set_credential(
            &state.db,
            &state.encryption_keys,
            &service.id,
            credential,
            USER_ID,
        )
        .await
        .expect("set ElevenLabs platform credential");
        service
    }

    async fn ensure_usage_index(db: &mongodb::Database) {
        db.collection::<mongodb::bson::Document>(PLATFORM_OP_USAGE)
            .create_index(
                IndexModel::builder()
                    .keys(mongodb::bson::doc! {
                        "op": 1,
                        "user_id": 1,
                        "yyyymmdd": 1,
                    })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create platform operation usage index");
    }

    async fn enable_platform_services_for_tests(db: &mongodb::Database) {
        crate::services::feature_flag_service::set_platform_override(
            db,
            crate::services::feature_flag_service::PLATFORM_SERVICES_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            USER_ID,
        )
        .await
        .expect("enable platform-services flag for test");
    }

    async fn spawn_twilio(status: StatusCode) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route(
            "/2010-04-01/Accounts/{account_sid}/Calls.json",
            post(move || async move {
                (
                    status,
                    Json(json!({
                        "sid": "CA22222222222222222222222222222222",
                        "status": "queued"
                    })),
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Twilio test server");
        let address = listener.local_addr().expect("Twilio test server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve Twilio test server");
        });
        (format!("http://{address}"), server)
    }

    async fn spawn_counted_twilio() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let forwarded = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/2010-04-01/Accounts/{account_sid}/Calls.json",
                post(|State(counter): State<Arc<AtomicUsize>>| async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::CREATED,
                        Json(json!({
                            "sid": "CA22222222222222222222222222222222",
                            "status": "queued"
                        })),
                    )
                }),
            )
            .with_state(forwarded.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind counted Twilio test server");
        let address = listener
            .local_addr()
            .expect("counted Twilio test server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve counted Twilio test server");
        });
        (format!("http://{address}"), forwarded, server)
    }

    async fn spawn_invalid_json_twilio() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route(
            "/2010-04-01/Accounts/{account_sid}/Calls.json",
            post(|| async {
                (
                    StatusCode::CREATED,
                    [(header::CONTENT_TYPE, "application/json")],
                    "not-json",
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind invalid JSON Twilio test server");
        let address = listener
            .local_addr()
            .expect("invalid JSON Twilio test server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve invalid JSON Twilio test server");
        });
        (format!("http://{address}"), server)
    }

    #[derive(Clone, Debug)]
    struct CapturedVendorRequest {
        method: Method,
        uri: String,
        authorization: Option<String>,
        elevenlabs_key: Option<String>,
        body: Vec<u8>,
    }

    async fn capture_vendor_request(
        State(captured): State<Arc<tokio::sync::Mutex<Vec<CapturedVendorRequest>>>>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: bytes::Bytes,
    ) -> impl IntoResponse {
        captured.lock().await.push(CapturedVendorRequest {
            method,
            uri: uri.to_string(),
            authorization: headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            elevenlabs_key: headers
                .get("xi-api-key")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            body: body.to_vec(),
        });
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({ "ok": true })),
        )
    }

    async fn spawn_capturing_vendor() -> (
        String,
        Arc<tokio::sync::Mutex<Vec<CapturedVendorRequest>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let captured = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let app = Router::new()
            .fallback(any(capture_vendor_request))
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind credential capture server");
        let address = listener.local_addr().expect("credential capture address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve credential capture server");
        });
        (format!("http://{address}"), captured, server)
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_own_connection(
        state: &AppState,
        catalog_slug: &str,
        endpoint_url: &str,
        auth_method: &str,
        auth_key_name: &str,
        credential_type: &str,
        credential: &str,
        oauth_access_token: bool,
    ) -> String {
        let existing_catalog = state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(mongodb::bson::doc! { "slug": catalog_slug, "is_active": true })
            .await
            .expect("look up existing catalog service");
        let catalog_id = existing_catalog
            .as_ref()
            .map(|service| service.id.clone())
            .unwrap_or_else(|| catalog_service_id(catalog_slug));
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = catalog_id.clone();
        catalog.slug = catalog_slug.to_string();
        catalog.name = catalog_slug.to_string();
        catalog.base_url = endpoint_url.to_string();
        catalog.service_category = "connection".to_string();
        catalog.requires_user_credential = true;
        catalog.auth_method = auth_method.to_string();
        catalog.auth_key_name = auth_key_name.to_string();
        catalog.credential_encrypted = Vec::new();
        if existing_catalog.is_none() {
            state
                .db
                .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
                .insert_one(catalog)
                .await
                .expect("insert user catalog service");
        }

        let endpoint = crate::test_utils::test_user_endpoint(
            &endpoint_id,
            USER_ID,
            catalog_slug,
            endpoint_url,
            None,
            Some(&catalog_id),
        );
        state
            .db
            .collection::<crate::models::user_endpoint::UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .expect("insert user endpoint");

        let encrypted = state
            .encryption_keys
            .encrypt(credential.as_bytes())
            .await
            .expect("encrypt own credential");
        let now = Utc::now();
        state
            .db
            .collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(UserApiKey {
                id: key_id.clone(),
                user_id: USER_ID.to_string(),
                label: catalog_slug.to_string(),
                credential_type: credential_type.to_string(),
                credential_encrypted: (!oauth_access_token).then_some(encrypted.clone()),
                access_token_encrypted: oauth_access_token.then_some(encrypted),
                refresh_token_encrypted: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_attempt_nonce: None,
                user_oauth_client_id_encrypted: None,
                user_oauth_client_secret_encrypted: None,
                credential_source: None,
                status: "active".to_string(),
                last_used_at: None,
                last_authorized_at: None,
                error_message: None,
                source: Some("user_created".to_string()),
                source_id: None,
                credential_epoch: 1,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert own credential");

        let mut user_service = crate::test_utils::test_user_service(
            &service_id,
            USER_ID,
            catalog_slug.trim_start_matches("api-"),
            &endpoint_id,
            Some(&catalog_id),
            None,
        );
        user_service.api_key_id = Some(key_id);
        user_service.auth_method = auth_method.to_string();
        user_service.auth_key_name = auth_key_name.to_string();
        state
            .db
            .collection::<UserService>(USER_SERVICES)
            .insert_one(user_service)
            .await
            .expect("insert own user service");
        service_id
    }

    async fn resolve_discovery_source_for_test(
        state: &AppState,
        auth: &AuthUser,
        operation: &PlatformOperation,
    ) -> (
        PlatformCredentialResolution,
        Option<crate::models::downstream_service::DownstreamService>,
    ) {
        let resolution_user_id = auth.proxy_resolution_user_id();
        let context = platform_operation_service::load_credential_resolution_context(
            &state.db,
            &resolution_user_id,
            std::slice::from_ref(operation),
        )
        .await
        .expect("load platform credential context");
        let vendor = context
            .service_by_slug(&operation.vendor_service_slug)
            .cloned();
        let descriptor = platform_operation_discovery_descriptor(operation.op);
        let caller = PlatformOperationCaller::from_auth_user(auth);
        let source = platform_operation_service::resolve_operation_credential_source(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &resolution_user_id,
            caller.credential_caller(),
            operation,
            platform_operation_service::CredentialResolutionMode::Discover {
                descriptor: &descriptor,
            },
            &context,
        )
        .await
        .expect("resolve platform credential source");
        (source, vendor)
    }

    #[tokio::test]
    async fn platform_services_flag_off_returns_not_found_for_all_http_operations() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_flag_default_off").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        let state = crate::test_utils::test_app_state(db);
        let auth_user = crate::test_utils::test_auth_user(USER_ID);

        let speak_result = speak(
            State(state.clone()),
            auth_user.clone(),
            billing_extension(),
            Json(SpeakRequest {
                text: "Hello".to_string(),
                voice_id: "voice".to_string(),
            }),
        )
        .await;
        assert!(matches!(speak_result, Err(AppError::NotFound(_))));

        let call_result = call_and_say(
            State(state),
            auth_user,
            billing_extension(),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
                from: None,
            }),
        )
        .await;
        assert!(matches!(call_result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn disabled_operation_returns_not_found() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_disabled").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::Speak,
                false,
                PlatformOperationConfig::Speak(SpeakConfig {
                    allowed_voice_ids: vec!["voice".to_string()],
                    max_chars: 1_000,
                    model_id: "eleven_multilingual_v2".to_string(),
                }),
            ))
            .await
            .expect("insert disabled operation");
        let result = speak(
            State(crate::test_utils::test_app_state(db)),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(SpeakRequest {
                text: "Hello".to_string(),
                voice_id: "voice".to_string(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn enabled_operation_with_missing_platform_credential_returns_bad_gateway_error() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_vendor_missing").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = catalog_service_id(platform_operation_service::SPEAK_VENDOR_SLUG);
        catalog.slug = platform_operation_service::SPEAK_VENDOR_SLUG.to_string();
        catalog.auth_method = "header".to_string();
        catalog.auth_key_name = "xi-api-key".to_string();
        catalog.credential_encrypted = Vec::new();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .expect("insert catalog service without platform credential");
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::Speak,
                true,
                PlatformOperationConfig::Speak(SpeakConfig {
                    allowed_voice_ids: vec!["voice".to_string()],
                    max_chars: 1_000,
                    model_id: "eleven_multilingual_v2".to_string(),
                }),
            ))
            .await
            .expect("insert enabled operation");
        let result = speak(
            State(crate::test_utils::test_app_state(db)),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(SpeakRequest {
                text: "Hello".to_string(),
                voice_id: "voice".to_string(),
            }),
        )
        .await;
        let error = result.expect_err("missing platform credential must fail closed");
        assert!(matches!(error, AppError::PlatformOperationUnavailable));
        assert_eq!(error.into_response().status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn daily_cap_allows_n_calls_and_rejects_n_plus_one() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_daily_cap").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        ensure_usage_index(&db).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let (base_url, server) = spawn_twilio(StatusCode::CREATED).await;
        insert_twilio_vendor(&state, base_url).await;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::CallAndSay,
                true,
                PlatformOperationConfig::CallAndSay(call_config(2)),
            ))
            .await
            .expect("insert call operation");

        for _ in 0..2 {
            let result = call_and_say(
                State(state.clone()),
                crate::test_utils::test_auth_user(USER_ID),
                billing_extension(),
                Json(CallAndSayRequest {
                    to: "+6512345678".to_string(),
                    message: "Hello".to_string(),
                    from: None,
                }),
            )
            .await;
            assert!(result.is_ok());
        }
        let result = call_and_say(
            State(state),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello again".to_string(),
                from: None,
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::RateLimited)));
        server.abort();
    }

    #[tokio::test]
    async fn vendor_failure_releases_daily_call_reservation() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_failed_call").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        ensure_usage_index(&db).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let (base_url, server) = spawn_twilio(StatusCode::INTERNAL_SERVER_ERROR).await;
        insert_twilio_vendor(&state, base_url).await;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::CallAndSay,
                true,
                PlatformOperationConfig::CallAndSay(call_config(2)),
            ))
            .await
            .expect("insert call operation");

        let result = call_and_say(
            State(state),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
                from: None,
            }),
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::PlatformOperationUnavailable)
        ));
        let usage = db
            .collection::<PlatformOpUsage>(PLATFORM_OP_USAGE)
            .find_one(mongodb::bson::doc! {
                "op": "call_and_say",
                "user_id": USER_ID,
            })
            .await
            .expect("read usage row");
        assert!(usage.is_none(), "failed calls must remove their quota row");
        server.abort();
    }

    #[tokio::test]
    async fn insufficient_credits_release_quota_before_credential_decryption_or_vendor_call() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_insufficient_credits").await
        else {
            eprintln!("skipping platform billing test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        crate::services::feature_flag_service::set_platform_override(
            &db,
            crate::services::feature_flag_service::BILLING_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            USER_ID,
        )
        .await
        .expect("enable billing rollout for test");
        ensure_usage_index(&db).await;
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                USER_ID,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert billing owner");

        let mut config = crate::test_utils::test_app_config();
        config.billing_enabled = true;
        config.billing_fail_closed = false;
        let mut state = crate::test_utils::test_app_state_with_config(db.clone(), config.clone());
        state.billing = Arc::new(crate::services::billing::BillingService::new_with_lago(
            db.clone(),
            Arc::new(config),
            Arc::new(crate::billing_integration_tests::FakeLago::default()),
        ));

        let (base_url, forwarded, server) = spawn_counted_twilio().await;
        let vendor = insert_twilio_vendor(&state, base_url).await;
        // Deliberately invalid ciphertext proves billing rejects before the
        // v2 platform credential is materialized.
        db.collection::<mongodb::bson::Document>(
            crate::models::platform_credential::COLLECTION_NAME,
        )
        .update_one(
            mongodb::bson::doc! { "catalog_service_id": &vendor.id },
            mongodb::bson::doc! {
                "$set": {
                    "credential_encrypted": mongodb::bson::Binary {
                        subtype: mongodb::bson::spec::BinarySubtype::Generic,
                        bytes: vec![1, 2, 3],
                    }
                }
            },
        )
        .await
        .expect("corrupt Twilio platform credential");
        let mut call_operation = operation(
            PlatformOperationName::CallAndSay,
            true,
            PlatformOperationConfig::CallAndSay(call_config(2)),
        );
        call_operation.billing = OperationBilling {
            metric: BillingMetric::Seconds,
            price_per_unit: "1".to_string(),
            base_fee_per_call: None,
            lago_metric_code: crate::services::billing::pricing::metric_code_for_operation(
                &vendor.slug,
                &call_operation.kind_key,
            ),
            sync_status: crate::models::service_billing::PricingSyncStatus::Synced,
            sync_error: None,
        };
        let operation_metric_code = call_operation.billing.lago_metric_code.clone();
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(call_operation)
            .await
            .expect("insert call operation");

        let now = Utc::now();
        db.collection::<crate::models::billing_wallet::BillingWallet>(
            crate::models::billing_wallet::COLLECTION_NAME,
        )
        .insert_one(crate::models::billing_wallet::BillingWallet {
            id: format!("wallet-{USER_ID}"),
            owner_id: USER_ID.to_string(),
            lago_customer_id: USER_ID.to_string(),
            lago_wallet_id: Some(format!("{USER_ID}:wallet")),
            lago_subscription_id: Some(format!("{USER_ID}:plan")),
            plan_kind: crate::models::billing_wallet::PlanKind::Prepaid,
            balance_credits: 0,
            reserved_credits: 0,
            pending_lago_debits: 0,
            pending_topup_expiry_credits: 0,
            has_payment_instrument: false,
            overdraft_cap_credits: 0,
            suspended: false,
            collection_state: crate::models::billing_wallet::CollectionState::Good,
            topup_expiry_checked_at: None,
            active_topup_expiry: None,
            balance_synced_at: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("insert empty prepaid wallet");
        db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
            crate::models::billing_rate_cache::COLLECTION_NAME,
        )
        .insert_one(crate::models::billing_rate_cache::BillingRateCache {
            id: crate::models::billing_rate_cache::BillingRateCache::cache_id(
                &operation_metric_code,
                None,
            ),
            lago_metric_code: operation_metric_code,
            model: None,
            credits_per_unit_micros: 1_000_000,
            synced_at: now,
        })
        .await
        .expect("insert request price");

        let decryption_probe = state.encryption_keys.clone();
        let decrypts_before = decryption_probe.decrypt_stats();
        let result = call_and_say(
            State(state),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
                from: None,
            }),
        )
        .await;
        assert_eq!(audit_outcome(&result), "insufficient_credits");
        assert!(matches!(result, Err(AppError::InsufficientCredits)));
        assert_eq!(decryption_probe.decrypt_stats(), decrypts_before);
        assert_eq!(forwarded.load(Ordering::SeqCst), 0);
        assert_eq!(
            db.collection::<PlatformOpUsage>(PLATFORM_OP_USAGE)
                .count_documents(mongodb::bson::doc! { "user_id": USER_ID })
                .await
                .expect("count quota rows"),
            0
        );
        assert_eq!(
            db.collection::<crate::models::usage_meter::UsageMeterRow>(
                crate::models::usage_meter::COLLECTION_NAME,
            )
            .count_documents(mongodb::bson::doc! { "billing_owner_id": USER_ID })
            .await
            .expect("count usage meters"),
            0
        );
        let wallet = db
            .collection::<crate::models::billing_wallet::BillingWallet>(
                crate::models::billing_wallet::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "owner_id": USER_ID })
            .await
            .expect("read wallet")
            .expect("wallet exists");
        assert_eq!(wallet.reserved_credits, 0);
        server.abort();
    }

    #[tokio::test]
    async fn invalid_success_payload_fails_meter_and_releases_wallet_and_daily_quota() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_invalid_success_payload").await
        else {
            eprintln!("skipping platform billing test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        crate::services::feature_flag_service::set_platform_override(
            &db,
            crate::services::feature_flag_service::BILLING_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            USER_ID,
        )
        .await
        .expect("enable billing rollout for test");
        ensure_usage_index(&db).await;
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                USER_ID,
                crate::models::user::UserType::Person,
            ))
            .await
            .expect("insert billing owner");

        let mut config = crate::test_utils::test_app_config();
        config.billing_enabled = true;
        config.billing_fail_closed = false;
        let mut state = crate::test_utils::test_app_state_with_config(db.clone(), config.clone());
        state.billing = Arc::new(crate::services::billing::BillingService::new_with_lago(
            db.clone(),
            Arc::new(config),
            Arc::new(crate::billing_integration_tests::FakeLago::default()),
        ));

        let (base_url, server) = spawn_invalid_json_twilio().await;
        let mut vendor = insert_twilio_vendor(&state, base_url).await;
        vendor.billing = Some(crate::models::service_billing::ServiceBilling {
            platform_billable: true,
            platform_metric: Some(crate::models::service_billing::BillingMetric::Requests),
            ..Default::default()
        });
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .replace_one(mongodb::bson::doc! { "_id": &vendor.id }, &vendor)
            .await
            .expect("make Twilio vendor billable");
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::CallAndSay,
                true,
                PlatformOperationConfig::CallAndSay(call_config(2)),
            ))
            .await
            .expect("insert call operation");

        let now = Utc::now();
        db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
            crate::models::billing_rate_cache::COLLECTION_NAME,
        )
        .insert_one(crate::models::billing_rate_cache::BillingRateCache {
            id: crate::models::billing_rate_cache::BillingRateCache::cache_id(
                "platform_requests",
                None,
            ),
            lago_metric_code: "platform_requests".to_string(),
            model: None,
            credits_per_unit_micros: 1_000_000,
            synced_at: now,
        })
        .await
        .expect("insert request price");

        let result = call_and_say(
            State(state),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
                from: None,
            }),
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::PlatformOperationUnavailable)
        ));
        assert_eq!(
            db.collection::<PlatformOpUsage>(PLATFORM_OP_USAGE)
                .count_documents(mongodb::bson::doc! { "user_id": USER_ID })
                .await
                .expect("count quota rows"),
            0
        );
        let meter = db
            .collection::<crate::models::usage_meter::UsageMeterRow>(
                crate::models::usage_meter::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "billing_owner_id": USER_ID })
            .await
            .expect("read usage meter")
            .expect("usage meter exists");
        assert_eq!(
            meter.status,
            crate::models::usage_meter::UsageStatus::Failed
        );
        assert!(meter.forwarded);
        assert!(meter.released);
        assert_eq!(meter.last_error.as_deref(), Some("vendor_response_invalid"));
        let wallet = db
            .collection::<crate::models::billing_wallet::BillingWallet>(
                crate::models::billing_wallet::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! { "owner_id": USER_ID })
            .await
            .expect("read wallet")
            .expect("wallet exists");
        assert_eq!(wallet.reserved_credits, 0);
        server.abort();
    }

    #[tokio::test]
    async fn own_connections_inject_auth_use_catalog_paths_and_skip_billing_and_quota() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_own_execution").await
        else {
            eprintln!("skipping platform own-connection test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        crate::services::feature_flag_service::set_platform_override(
            &db,
            crate::services::feature_flag_service::BILLING_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            USER_ID,
        )
        .await
        .expect("enable billing rollout for own-connection test");
        let mut config = crate::test_utils::test_app_config();
        config.billing_enabled = true;
        config.billing_fail_closed = true;
        let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
        let (base_url, captured, server) = spawn_capturing_vendor().await;

        insert_own_connection(
            &state,
            "api-elevenlabs",
            &base_url,
            "header",
            "xi-api-key",
            "api_key",
            "elevenlabs-own-key",
            false,
        )
        .await;
        let own_twilio_sid = format!("AC{}", "a".repeat(32));
        let twilio_credential = format!("{own_twilio_sid}:auth:token-with-colon");
        insert_own_connection(
            &state,
            "api-twilio",
            &base_url,
            "basic",
            "Authorization",
            "basic",
            &twilio_credential,
            false,
        )
        .await;

        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_many([
                operation(
                    PlatformOperationName::Speak,
                    true,
                    PlatformOperationConfig::Speak(SpeakConfig {
                        allowed_voice_ids: vec!["platform-voice".to_string()],
                        max_chars: 100,
                        model_id: "eleven_multilingual_v2".to_string(),
                    }),
                ),
                operation(
                    PlatformOperationName::CallAndSay,
                    true,
                    PlatformOperationConfig::CallAndSay(CallAndSayConfig {
                        allowed_destination_prefixes: vec!["+1".to_string()],
                        ..call_config(1)
                    }),
                ),
            ])
            .await
            .expect("insert own-connection operations");

        let speak_response = speak(
            State(state.clone()),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(SpeakRequest {
                text: "Hello from an own connection".to_string(),
                voice_id: "voice-own".to_string(),
            }),
        )
        .await
        .expect("execute own speech");
        assert_eq!(
            speak_response.headers()[CREDENTIAL_SOURCE_HEADER],
            PlatformCredentialSource::OwnConnection.as_str()
        );

        let call_response = call_and_say(
            State(state),
            crate::test_utils::test_auth_user(USER_ID),
            billing_extension(),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Own Twilio call".to_string(),
                from: Some("+14155550123".to_string()),
            }),
        )
        .await
        .expect("execute own Twilio call");
        assert_eq!(
            call_response.headers()[CREDENTIAL_SOURCE_HEADER],
            PlatformCredentialSource::OwnConnection.as_str()
        );

        let requests = captured.lock().await.clone();
        assert_eq!(requests.len(), 2);
        let speak_request = &requests[0];
        assert_eq!(speak_request.method, Method::POST);
        assert_eq!(speak_request.uri, "/v1/text-to-speech/voice-own");
        assert_eq!(
            speak_request.elevenlabs_key.as_deref(),
            Some("elevenlabs-own-key")
        );

        let call_request = &requests[1];
        assert_eq!(call_request.method, Method::POST);
        assert_eq!(
            call_request.uri,
            format!("/2010-04-01/Accounts/{own_twilio_sid}/Calls.json")
        );
        assert_eq!(
            call_request.authorization.as_deref(),
            Some(
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(twilio_credential)
                )
                .as_str()
            )
        );
        let call_form = url::form_urlencoded::parse(&call_request.body)
            .into_owned()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(call_form.get("To").map(String::as_str), Some("+6512345678"));
        assert_eq!(
            call_form.get("From").map(String::as_str),
            Some("+14155550123")
        );
        assert_eq!(call_form.get("TimeLimit").map(String::as_str), Some("600"));

        assert_eq!(
            db.collection::<mongodb::bson::Document>(crate::models::usage_meter::COLLECTION_NAME,)
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count own-connection meters"),
            0
        );
        assert_eq!(
            db.collection::<mongodb::bson::Document>(PLATFORM_OP_USAGE)
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count own-connection quota rows"),
            0
        );
        assert_eq!(
            db.collection::<mongodb::bson::Document>(
                crate::models::billing_wallet::COLLECTION_NAME,
            )
            .count_documents(mongodb::bson::doc! {})
            .await
            .expect("count own-connection wallets"),
            0
        );
        server.abort();
    }

    #[tokio::test]
    async fn scoped_agent_treats_out_of_scope_connection_as_platform_and_audits_decision() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_agent_scope_fallback").await
        else {
            eprintln!("skipping platform agent-scope test: no local MongoDB available");
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let (base_url, captured, server) = spawn_capturing_vendor().await;
        insert_speak_vendor(&state, base_url.clone(), "platform-speak-secret").await;
        let user_service_id = insert_own_connection(
            &state,
            "api-elevenlabs",
            &base_url,
            "header",
            "xi-api-key",
            "api_key",
            "own-speak-secret",
            false,
        )
        .await;
        let operation = operation(
            PlatformOperationName::Speak,
            true,
            PlatformOperationConfig::Speak(SpeakConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                max_chars: 1_000,
                model_id: "eleven_multilingual_v2".to_string(),
            }),
        );
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation)
            .await
            .expect("insert scoped speak operation");
        let caller = PlatformOperationCaller {
            actor_user_id: USER_ID.to_string(),
            resolution_user_id: USER_ID.to_string(),
            api_key_id: Some("agent-key".to_string()),
            auth_method: AuthMethod::ApiKey,
            acting_client_id: None,
            allow_all_services: false,
            allowed_service_ids: Vec::new(),
        };
        let result = execute_speak_for_caller(
            &state,
            &caller,
            SpeakRequest {
                text: "nyxid".to_string(),
                voice_id: "voice-a".to_string(),
            },
            BillingIngress::PlatformOperation,
            enforce_platform_billing_classification(BillingRoutePolicy::Metered(
                BillingIngress::PlatformOperation,
            ))
            .expect("platform billing classification"),
        )
        .await;
        let execution = result.as_ref().expect("execute platform fallback");
        assert_eq!(
            execution.credential_source,
            PlatformCredentialSource::Platform
        );
        assert!(execution.own_connection_out_of_scope);
        let audit = platform_operation_audit_metadata("speak", &result, Instant::now(), json!({}));
        assert_eq!(audit["own_connection_out_of_scope"], Value::Bool(true));
        assert_eq!(
            audit["credential_source"],
            Value::String("platform".to_string())
        );

        let requests = captured.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].uri, "/v1/text-to-speech/voice-a");
        assert_eq!(
            requests[0].elevenlabs_key.as_deref(),
            Some("platform-speak-secret")
        );
        assert_ne!(
            requests[0].elevenlabs_key.as_deref(),
            Some("own-speak-secret")
        );
        assert!(!caller.allowed_service_ids.contains(&user_service_id));
        server.abort();
    }

    #[tokio::test]
    async fn agent_binding_override_is_injected_for_own_platform_operation() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_agent_binding").await
        else {
            eprintln!("skipping platform agent-binding test: no local MongoDB available");
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let (base_url, captured, server) = spawn_capturing_vendor().await;
        let user_service_id = insert_own_connection(
            &state,
            "api-elevenlabs",
            &base_url,
            "header",
            "xi-api-key",
            "api_key",
            "default-speak-secret",
            false,
        )
        .await;
        let default_key_id = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(mongodb::bson::doc! { "_id": &user_service_id })
            .await
            .expect("read own service")
            .and_then(|service| service.api_key_id)
            .expect("own service key id");
        let mut bound_key = db
            .collection::<UserApiKey>(USER_API_KEYS)
            .find_one(mongodb::bson::doc! { "_id": default_key_id })
            .await
            .expect("read default key")
            .expect("default key exists");
        bound_key.id = uuid::Uuid::new_v4().to_string();
        bound_key.label = "bound ElevenLabs credential".to_string();
        bound_key.credential_encrypted = Some(
            state
                .encryption_keys
                .encrypt(b"bound-speak-secret")
                .await
                .expect("encrypt bound key"),
        );
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(&bound_key)
            .await
            .expect("insert bound key");
        let now = Utc::now();
        db.collection::<crate::models::agent_service_binding::AgentServiceBinding>(
            crate::models::agent_service_binding::COLLECTION_NAME,
        )
        .insert_one(crate::models::agent_service_binding::AgentServiceBinding {
            id: uuid::Uuid::new_v4().to_string(),
            api_key_id: "agent-key".to_string(),
            user_service_id: user_service_id.clone(),
            user_api_key_id: bound_key.id,
            user_id: USER_ID.to_string(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("insert agent binding");
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::Speak,
                true,
                PlatformOperationConfig::Speak(SpeakConfig {
                    allowed_voice_ids: vec!["voice-a".to_string()],
                    max_chars: 1_000,
                    model_id: "eleven_multilingual_v2".to_string(),
                }),
            ))
            .await
            .expect("insert bound speak operation");
        let caller = PlatformOperationCaller {
            actor_user_id: USER_ID.to_string(),
            resolution_user_id: USER_ID.to_string(),
            api_key_id: Some("agent-key".to_string()),
            auth_method: AuthMethod::ApiKey,
            acting_client_id: None,
            allow_all_services: false,
            allowed_service_ids: vec![user_service_id],
        };
        let execution = execute_speak_for_caller(
            &state,
            &caller,
            SpeakRequest {
                text: "binding".to_string(),
                voice_id: "voice-a".to_string(),
            },
            BillingIngress::PlatformOperation,
            enforce_platform_billing_classification(BillingRoutePolicy::Metered(
                BillingIngress::PlatformOperation,
            ))
            .expect("platform billing classification"),
        )
        .await
        .expect("execute with bound credential");
        assert_eq!(
            execution.credential_source,
            PlatformCredentialSource::OwnConnection
        );
        let requests = captured.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].elevenlabs_key.as_deref(),
            Some("bound-speak-secret")
        );
        server.abort();
    }

    #[tokio::test]
    async fn own_connection_approval_fails_closed_for_agent_but_session_bypasses() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_own_approval").await
        else {
            eprintln!("skipping platform approval test: no local MongoDB available");
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let (base_url, captured, server) = spawn_capturing_vendor().await;
        let user_service_id = insert_own_connection(
            &state,
            "api-elevenlabs",
            &base_url,
            "header",
            "xi-api-key",
            "api_key",
            "approval-speak-secret",
            false,
        )
        .await;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::Speak,
                true,
                PlatformOperationConfig::Speak(SpeakConfig {
                    allowed_voice_ids: vec!["voice-a".to_string()],
                    max_chars: 1_000,
                    model_id: "eleven_multilingual_v2".to_string(),
                }),
            ))
            .await
            .expect("insert approval speak operation");
        let now = Utc::now();
        db.collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: USER_ID.to_string(),
                service_id: user_service_id.clone(),
                service_name: "ElevenLabs".to_string(),
                approval_required: true,
                approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
                rules: Vec::new(),
                default_effect: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .expect("insert approval policy");
        let agent = PlatformOperationCaller {
            actor_user_id: USER_ID.to_string(),
            resolution_user_id: USER_ID.to_string(),
            api_key_id: Some("agent-key".to_string()),
            auth_method: AuthMethod::ApiKey,
            acting_client_id: None,
            allow_all_services: false,
            allowed_service_ids: vec![user_service_id],
        };
        let permit = || {
            enforce_platform_billing_classification(BillingRoutePolicy::Metered(
                BillingIngress::PlatformOperation,
            ))
            .expect("platform billing classification")
        };
        let error = match execute_speak_for_caller(
            &state,
            &agent,
            SpeakRequest {
                text: "approval".to_string(),
                voice_id: "voice-a".to_string(),
            },
            BillingIngress::PlatformOperation,
            permit(),
        )
        .await
        {
            Ok(_) => panic!("agent request must require approval"),
            Err(error) => error,
        };
        assert!(matches!(
            &error,
            AppError::PlatformOperationApprovalRequired { .. }
        ));
        assert_eq!(error.error_code(), 11804);
        assert_eq!(error.error_key(), "platform_operation_approval_required");
        assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
        assert!(captured.lock().await.is_empty());
        assert_eq!(
            db.collection::<mongodb::bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
                .count_documents(mongodb::bson::doc! {})
                .await
                .expect("count approval-blocked billing rows"),
            0
        );

        let session = PlatformOperationCaller {
            actor_user_id: USER_ID.to_string(),
            resolution_user_id: USER_ID.to_string(),
            api_key_id: None,
            auth_method: AuthMethod::Session,
            acting_client_id: None,
            allow_all_services: true,
            allowed_service_ids: Vec::new(),
        };
        execute_speak_for_caller(
            &state,
            &session,
            SpeakRequest {
                text: "session".to_string(),
                voice_id: "voice-a".to_string(),
            },
            BillingIngress::PlatformOperation,
            permit(),
        )
        .await
        .expect("session bypasses request approval");
        assert_eq!(captured.lock().await.len(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn discovery_reports_shared_resolver_states_and_operation_price() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_discovery_states").await
        else {
            eprintln!("skipping platform discovery test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        crate::services::feature_flag_service::set_platform_override(
            &db,
            crate::services::feature_flag_service::BILLING_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            USER_ID,
        )
        .await
        .expect("enable billing rollout for discovery test");
        let state = crate::test_utils::test_app_state(db.clone());
        let mut vendor = crate::models::downstream_service::test_helpers::dummy_service();
        vendor.id = catalog_service_id(platform_operation_service::SPEAK_VENDOR_SLUG);
        vendor.slug = platform_operation_service::SPEAK_VENDOR_SLUG.to_string();
        vendor.name = "Platform ElevenLabs".to_string();
        vendor.base_url = "https://api.elevenlabs.io".to_string();
        vendor.service_category = "internal".to_string();
        vendor.visibility = "public".to_string();
        vendor.requires_user_credential = false;
        vendor.auth_method = "header".to_string();
        vendor.auth_key_name = "xi-api-key".to_string();
        vendor.credential_encrypted = Vec::new();
        vendor.billing = Some(crate::models::service_billing::ServiceBilling {
            platform_billable: true,
            platform_metric: Some(crate::models::service_billing::BillingMetric::Requests),
            platform_pricing: Some(crate::models::service_billing::ServicePlatformPricing {
                credits_per_unit: "9.99".to_string(),
                lago_metric_code: "platform_svc_platform-elevenlabs".to_string(),
                sync_status: crate::models::service_billing::PricingSyncStatus::Synced,
                sync_error: None,
            }),
            ..Default::default()
        });
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&vendor)
            .await
            .expect("insert priced discovery vendor");
        crate::services::platform_credential_service::set_credential(
            &db,
            &state.encryption_keys,
            &vendor.id,
            "platform-elevenlabs-key",
            USER_ID,
        )
        .await
        .expect("set discovery platform credential");
        let mut speak_operation = operation(
            PlatformOperationName::Speak,
            true,
            PlatformOperationConfig::Speak(SpeakConfig {
                allowed_voice_ids: vec!["platform-voice".to_string()],
                max_chars: 100,
                model_id: "eleven_multilingual_v2".to_string(),
            }),
        );
        speak_operation.billing = OperationBilling {
            metric: BillingMetric::Characters,
            price_per_unit: "0.25".to_string(),
            base_fee_per_call: None,
            lago_metric_code: crate::services::billing::pricing::metric_code_for_operation(
                &vendor.slug,
                &speak_operation.kind_key,
            ),
            sync_status: crate::models::service_billing::PricingSyncStatus::Synced,
            sync_error: None,
        };
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(speak_operation)
            .await
            .expect("insert discovery operation");
        let operation_row =
            platform_operation_service::load_enabled_operation(&db, PlatformOperationName::Speak)
                .await
                .expect("read discovery operation");
        let auth = crate::test_utils::test_auth_user(USER_ID);

        let no_connection = list_operations(State(state.clone()), auth.clone())
            .await
            .expect("discover platform source");
        assert_eq!(no_connection.operations.len(), 1);
        assert_eq!(
            no_connection.operations[0].credential_source,
            PlatformCredentialSource::Platform
        );
        assert!(no_connection.operations[0].own_connection.is_none());
        assert!(no_connection.operations[0].pricing.billable);
        assert_eq!(no_connection.operations[0].pricing.metric, "characters");
        // A per-character price must render as such. Before the contract fix
        // this surface could only express a per-call price.
        assert_eq!(
            no_connection.operations[0].pricing.display,
            "0.25 credits per character"
        );
        let (platform_source, platform_vendor) =
            resolve_discovery_source_for_test(&state, &auth, &operation_row).await;
        assert_eq!(
            platform_tool_credential_sentence(
                &operation_row,
                &platform_source,
                platform_vendor.as_ref(),
                true,
            ),
            "Uses the platform credential (0.25 credits per character). Allowed voice ids: platform-voice."
        );

        let user_service_id = insert_own_connection(
            &state,
            "api-elevenlabs",
            "https://api.elevenlabs.io",
            "header",
            "xi-api-key",
            "api_key",
            "own-elevenlabs-key",
            false,
        )
        .await;
        let own = list_operations(State(state.clone()), auth.clone())
            .await
            .expect("discover own source");
        assert_eq!(
            own.operations[0].credential_source,
            PlatformCredentialSource::OwnConnection
        );
        assert!(
            own.operations[0]
                .own_connection
                .as_ref()
                .is_some_and(|connection| connection.usable && connection.reason.is_none())
        );
        let (own_source, own_vendor) =
            resolve_discovery_source_for_test(&state, &auth, &operation_row).await;
        assert_eq!(
            platform_tool_credential_sentence(
                &operation_row,
                &own_source,
                own_vendor.as_ref(),
                true,
            ),
            "Uses your connected ElevenLabs account."
        );

        let scoped_agent_key_id = uuid::Uuid::new_v4().to_string();
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(ApiKey {
                id: scoped_agent_key_id.clone(),
                user_id: USER_ID.to_string(),
                name: "Scoped platform agent".to_string(),
                key_prefix: "nyxid_ag_test".to_string(),
                key_hash: "deadbeef".repeat(8),
                scopes: "proxy".to_string(),
                last_used_at: None,
                expires_at: None,
                is_active: true,
                created_at: Utc::now(),
                rotation_predecessor_id: None,
                state_version: 1,
                updated_at: Some(Utc::now()),
                description: None,
                allowed_service_ids: Vec::new(),
                allowed_node_ids: Vec::new(),
                allow_all_services: false,
                allow_all_nodes: false,
                rate_limit_per_second: None,
                rate_limit_burst: None,
                platform: Some("test".to_string()),
                callback_url: None,
                purpose: Default::default(),
                scheduled_write_enabled: false,
            })
            .await
            .expect("insert scoped platform agent key");
        let mut scoped_agent_auth = auth.clone();
        scoped_agent_auth.auth_method = AuthMethod::ApiKey;
        scoped_agent_auth.api_key_id = Some(scoped_agent_key_id);
        scoped_agent_auth.allow_all_services = false;
        scoped_agent_auth.allowed_service_ids.clear();
        let out_of_scope = list_operations(State(state.clone()), scoped_agent_auth.clone())
            .await
            .expect("discover out-of-scope own connection as platform source");
        assert_eq!(
            out_of_scope.operations[0].credential_source,
            PlatformCredentialSource::Platform
        );
        assert!(out_of_scope.operations[0].own_connection.is_none());
        let (out_of_scope_source, out_of_scope_vendor) =
            resolve_discovery_source_for_test(&state, &scoped_agent_auth, &operation_row).await;
        assert_eq!(
            platform_tool_credential_sentence(
                &operation_row,
                &out_of_scope_source,
                out_of_scope_vendor.as_ref(),
                true,
            ),
            "Uses the platform credential (0.25 credits per character). Allowed voice ids: platform-voice."
        );

        let now = Utc::now();
        db.collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: USER_ID.to_string(),
                service_id: user_service_id.clone(),
                service_name: "ElevenLabs".to_string(),
                approval_required: true,
                approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
                rules: Vec::new(),
                default_effect: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .expect("insert discovery approval policy");
        let mut agent_auth = scoped_agent_auth.clone();
        agent_auth.allow_all_services = true;
        let approval_required = list_operations(State(state.clone()), agent_auth.clone())
            .await
            .expect("discover approval-required source");
        assert!(
            approval_required.operations[0]
                .own_connection
                .as_ref()
                .is_some_and(|connection| !connection.usable
                    && connection.reason == Some("approval_required"))
        );
        let (approval_source, approval_vendor) =
            resolve_discovery_source_for_test(&state, &agent_auth, &operation_row).await;
        assert_eq!(
            platform_tool_credential_sentence(
                &operation_row,
                &approval_source,
                approval_vendor.as_ref(),
                true,
            ),
            "Your ElevenLabs connection requires approval per request; calls fail until you approve-free or disable it."
        );
        db.collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .delete_many(mongodb::bson::doc! {})
        .await
        .expect("remove discovery approval policy");

        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": &user_service_id },
                mongodb::bson::doc! { "$set": { "is_active": false } },
            )
            .await
            .expect("disable own discovery connection");
        let disabled = list_operations(State(state.clone()), auth.clone())
            .await
            .expect("discover disabled source");
        assert_eq!(
            disabled.operations[0].credential_source,
            PlatformCredentialSource::Platform
        );
        assert!(
            disabled.operations[0]
                .own_connection
                .as_ref()
                .is_some_and(|connection| !connection.usable
                    && !connection.is_active
                    && connection.reason == Some("disabled"))
        );

        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": &user_service_id },
                mongodb::bson::doc! {
                    "$set": { "is_active": true, "node_id": "node-1" }
                },
            )
            .await
            .expect("make discovery connection node-routed");
        let node_routed = list_operations(State(state.clone()), auth.clone())
            .await
            .expect("discover node source");
        assert!(
            node_routed.operations[0]
                .own_connection
                .as_ref()
                .is_some_and(
                    |connection| !connection.usable && connection.reason == Some("node_routed")
                )
        );
        let (node_source, node_vendor) =
            resolve_discovery_source_for_test(&state, &auth, &operation_row).await;
        assert_eq!(
            platform_tool_credential_sentence(
                &operation_row,
                &node_source,
                node_vendor.as_ref(),
                true,
            ),
            "Your ElevenLabs connection is node-routed; calls fail until you disable it or connect a server-held key."
        );

        let api_key_id = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(mongodb::bson::doc! { "_id": &user_service_id })
            .await
            .expect("read discovery connection")
            .and_then(|service| service.api_key_id)
            .expect("discovery connection has key");
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                mongodb::bson::doc! { "_id": &user_service_id },
                mongodb::bson::doc! { "$unset": { "node_id": "" } },
            )
            .await
            .expect("remove discovery node route");
        db.collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                mongodb::bson::doc! { "_id": api_key_id },
                mongodb::bson::doc! { "$set": { "status": "revoked" } },
            )
            .await
            .expect("revoke discovery credential");
        let unusable = list_operations(State(state.clone()), auth.clone())
            .await
            .expect("discover unusable source");
        assert!(
            unusable.operations[0]
                .own_connection
                .as_ref()
                .is_some_and(
                    |connection| !connection.usable && connection.reason == Some("unusable")
                )
        );
        let (unusable_source, unusable_vendor) =
            resolve_discovery_source_for_test(&state, &auth, &operation_row).await;
        assert_eq!(
            platform_tool_credential_sentence(
                &operation_row,
                &unusable_source,
                unusable_vendor.as_ref(),
                true,
            ),
            "Your ElevenLabs connection is unusable; reconnect or disable it."
        );
    }

    #[tokio::test]
    async fn delegated_relay_and_service_account_callers_are_rejected() {
        let state = Arc::new(crate::test_utils::test_app_state_no_db().await);
        for method in [
            AuthMethod::Delegated,
            AuthMethod::Relay,
            AuthMethod::ServiceAccount,
        ] {
            let mut auth_user = crate::test_utils::test_auth_user(USER_ID);
            auth_user.auth_method = method;
            assert!(matches!(
                ensure_platform_operation_caller(&state, &auth_user).await,
                Err(AppError::Forbidden(_))
            ));
        }
    }

    #[test]
    fn audit_payloads_are_metadata_only() {
        let result: AppResult<()> = Ok(());
        assert_eq!(audit_outcome(&result), "succeeded");
        let unusable: AppResult<()> = Err(AppError::PlatformOperationOwnConnectionUnusable {
            vendor: "X".to_string(),
            detail: "The credential is unavailable.".to_string(),
        });
        assert_eq!(audit_outcome(&unusable), "own_connection_unusable");
        let approval_required: AppResult<()> = Err(AppError::PlatformOperationApprovalRequired {
            vendor: "X".to_string(),
        });
        assert_eq!(audit_outcome(&approval_required), "approval_required");
        let invalid_shape: AppResult<()> = Err(AppError::BadRequest(
            "query must contain between 1 and 512 characters.".to_string(),
        ));
        assert_eq!(audit_outcome(&invalid_shape), "rejected");
        let call_metadata = json!({
            "message_chars": 12,
            "destination_suffix": "***5678",
        });
        let encoded = serde_json::to_string(&call_metadata).expect("encode audit metadata");
        assert!(!encoded.contains("Hello world"));
        assert!(!encoded.contains("+6512345678"));
    }
}
