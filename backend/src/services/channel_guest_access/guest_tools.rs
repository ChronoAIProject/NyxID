use bson::doc;
use mongodb::{ClientSession, Database};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{GuestTurn, google_sheets, personal_records, policy};
use crate::{
    errors::{AppError, AppResult},
    models::{notification_channel, service_approval_config::ApprovalVerb},
    services::{
        approval_policy::GlobalApprovalMode,
        approval_service,
        assistant_acknowledgement_service::{self as acks, ChatAuthority},
        mcp_service::{McpToolDefinition, McpToolEndpoint, McpToolService, McpToolSource},
        proxy_authorization::CanonicalPath,
        proxy_service::ProxyTarget,
        transaction_service,
    },
};

fn verb(action: &str) -> ApprovalVerb {
    match action {
        "read" | "list" => ApprovalVerb::Read,
        "delete" => ApprovalVerb::Destructive,
        _ => ApprovalVerb::Write,
    }
}

fn records_action(tool_name: &str) -> &str {
    tool_name
        .strip_prefix("nyxid__personal_records_")
        .unwrap_or("")
}

pub(crate) async fn definitions(
    db: &Database,
    chat: &ChatAuthority,
    approval_owner_id: &str,
) -> AppResult<Vec<McpToolDefinition>> {
    let turn = chat.channel_guest.as_ref().ok_or_else(policy::denied)?;
    definitions_for_turn(db, turn, approval_owner_id).await
}

pub(crate) async fn definitions_for_turn(
    db: &Database,
    turn: &GuestTurn,
    approval_owner_id: &str,
) -> AppResult<Vec<McpToolDefinition>> {
    let Some(policy) = policy::load_bound_policy(db, turn).await? else {
        return Ok(Vec::new());
    };
    let mode = approval_service::user_approval_mode(db, approval_owner_id).await?;
    let mut definitions = Vec::new();
    for mut tool in personal_records::tool_definitions() {
        let action = records_action(&tool.name);
        if mode.requires(&verb(action)) {
            continue;
        }
        let Ok(permit) = policy::authorize(&policy, turn, &personal_request(action, None, &[]))
        else {
            continue;
        };
        let Some(properties) = tool.input_schema["properties"].as_object_mut() else {
            continue;
        };
        if matches!(action, "create" | "update") {
            let mut any_field_allowed = false;
            for field in personal_records::FIELDS {
                let fields = [field];
                let request = personal_request(action, None, &fields);
                if policy::authorize(&policy, turn, &request).is_ok() {
                    any_field_allowed = true;
                } else {
                    properties.remove(field);
                }
            }
            if !any_field_allowed {
                continue;
            }
        } else if action == "list" && !permit.read_fields().iter().any(|name| name == "title") {
            properties.remove("title_prefix");
        }
        definitions.push(tool);
    }
    Ok(definitions)
}

fn personal_request<'a>(
    action: &'a str,
    record_id: Option<&'a str>,
    fields: &'a [&'a str],
) -> policy::GuestDataRequest<'a> {
    policy::GuestDataRequest {
        source_id: personal_records::SOURCE_ID,
        collection_id: personal_records::SOURCE_ID,
        action,
        record_id,
        record_owner_id: None,
        fields,
    }
}

pub(crate) async fn execute(
    db: &Database,
    chat: &ChatAuthority,
    approval_owner_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> AppResult<Value> {
    let turn = chat.channel_guest.as_ref().ok_or_else(policy::denied)?;
    policy::load_bound_policy(db, turn)
        .await?
        .ok_or_else(policy::denied)?;
    let context = (db, chat, approval_owner_id, tool_name, arguments);
    transaction_service::run_db_only(db, &context, |session, context| {
        let (db, chat, approval_owner_id, tool_name, arguments) = *context;
        Box::pin(execute_in_session(
            db,
            session,
            chat,
            approval_owner_id,
            tool_name,
            arguments,
        ))
    })
    .await
}

async fn execute_in_session(
    db: &Database,
    session: &mut ClientSession,
    chat: &ChatAuthority,
    approval_owner_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> AppResult<Value> {
    let turn = chat.channel_guest.as_ref().ok_or_else(policy::denied)?;
    acks::fence(db, chat, session).await?;
    execute_records_in_session(db, session, turn, approval_owner_id, tool_name, arguments).await
}

/// The caller fences the original hosted turn or relay event and its
/// credential inside this transaction before invoking a local effect.
pub(crate) async fn execute_records_in_session(
    db: &Database,
    session: &mut ClientSession,
    turn: &GuestTurn,
    approval_owner_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> AppResult<Value> {
    let request = personal_records::parse(tool_name, arguments)?;
    let policy = policy::fence_policy_in_session(db, session, turn).await?;
    let fields: Vec<_> = request.fields().collect();
    let permit = policy::authorize(
        &policy,
        turn,
        &personal_request(request.action(), request.record_id(), &fields),
    )?;
    // Lock an existing preference in the effect transaction so switching to
    // All Requests cannot race the commit of a record effect.
    let preference = db
        .collection::<notification_channel::NotificationChannel>(
            notification_channel::COLLECTION_NAME,
        )
        .find_one_and_update(
            doc! { "user_id": approval_owner_id },
            doc! { "$set": { "guest_data_approval_fence": uuid::Uuid::new_v4().to_string() } },
        )
        .session(&mut *session)
        .await?
        .and_then(|channel| channel.approval_required);
    if GlobalApprovalMode::from_stored(preference).requires(&verb(request.action())) {
        return Err(AppError::Forbidden(
            "The owner's approval settings require approval for this operation".into(),
        ));
    }
    personal_records::execute(db, session, &permit, turn.meta_grant_id.as_deref(), request).await
}

pub(crate) fn sheets_endpoint_action(endpoint: &McpToolEndpoint) -> Option<&'static str> {
    if endpoint.async_operation.is_some() {
        return None;
    }
    match (endpoint.name.as_str(), endpoint.method.as_str()) {
        ("sheets_get_values", "GET") => Some("read"),
        ("sheets_append_values", "POST") => Some("create"),
        _ => None,
    }
}

pub(crate) async fn filter_catalog(
    db: &Database,
    turn: &GuestTurn,
    services: &mut Vec<McpToolService>,
) -> AppResult<()> {
    let policy = policy::load_bound_policy(db, turn).await?;
    services.retain_mut(|service| {
        let Some(policy) = &policy else { return false };
        let McpToolSource::UserManaged {
            user_service_id,
            catalog_service_id: Some(_),
            node_id: None,
            ..
        } = &service.source
        else {
            return false;
        };
        let metadata = &service.durable_endpoint_metadata;
        service.endpoints.retain(|endpoint| {
            metadata
                .get(&endpoint.endpoint_id)
                .is_some_and(|m| m.catalog_contract)
                && sheets_endpoint_action(endpoint).is_some_and(|action| {
                    policy::has_sheets_grant(policy, turn, user_service_id, action)
                })
        });
        !service.endpoints.is_empty()
    });
    Ok(())
}

pub(crate) struct SheetsRequest<'a> {
    pub catalog_slug: Option<&'a str>,
    pub connection_id: Option<&'a str>,
    pub target: &'a ProxyTarget,
    pub method: &'a reqwest::Method,
    pub path: &'a CanonicalPath,
    pub query: Option<&'a str>,
    pub body: &'a [u8],
    pub content_type: Option<&'a str>,
}

/// The ordinary approval identities and descriptor for the exact admitted
/// Sheets operation. Re-evaluate policy before external effects without
/// creating approval requests or consuming someone else's grant.
pub(crate) struct SheetsApproval {
    pub(crate) actor: String,
    pub(crate) owner: String,
    pub(crate) catalog: String,
    pub(crate) descriptor: crate::services::operation_descriptor::OperationDescriptor,
    pub(crate) auto_connected: bool,
}

/// The operation can only be minted from a concrete hosted destination and
/// an exact canonical request. Recheck against live policy at final egress.
pub(crate) struct AuthorizedSheets {
    pub action: &'static str,
    pub(crate) approval: Option<SheetsApproval>,
    operation: google_sheets::SheetsOperation,
    turn: GuestTurn,
    catalog_slug: String,
    connection_id: String,
    path: CanonicalPath,
    query: Option<String>,
    body_hash: [u8; 32],
}

impl AuthorizedSheets {
    pub(crate) async fn revalidate(&self, db: &Database, chat: &ChatAuthority) -> AppResult<()> {
        let turn = chat.channel_guest.as_ref().ok_or_else(policy::denied)?;
        if turn != &self.turn {
            return Err(policy::denied());
        }
        chat.ensure_live_turn(db, None, None, "assistant.channel_guest_sheets")
            .await?;
        authorize_sheets_operation(db, turn, &self.operation).await
    }

    pub(crate) async fn before_send(
        &self,
        db: &Database,
        chat: &ChatAuthority,
        request: &reqwest::Request,
    ) -> AppResult<()> {
        self.validate_final_request(request)?;
        let approval = self.approval.as_ref().ok_or_else(policy::denied)?;
        let decision = approval_service::resolve_org_aware_approval(
            db,
            &approval.actor,
            &approval.owner,
            &approval.catalog,
            &approval.descriptor,
            approval.auto_connected,
        )
        .await?;
        if decision.required
            || decision.effect == crate::models::service_approval_config::ApprovalEffect::Deny
        {
            return Err(policy::denied());
        }
        self.revalidate(db, chat).await
    }

    fn validate_final_request(&self, request: &reqwest::Request) -> AppResult<()> {
        let url = request.url();
        let headers = request.headers();
        if url.scheme() != "https"
            || url.host_str() != Some("sheets.googleapis.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || headers.contains_key(reqwest::header::CONTENT_ENCODING)
            || headers
                .get_all(reqwest::header::CONTENT_TYPE)
                .iter()
                .count()
                > 1
        {
            return Err(policy::denied());
        }
        let path = CanonicalPath::from_mcp_built(url.path())?;
        let body = match request.body() {
            Some(body) => body.as_bytes().ok_or_else(policy::denied)?,
            None => &[],
        };
        let body_hash: [u8; 32] = Sha256::digest(body).into();
        if path != self.path
            || body_hash != self.body_hash
            || nyxid_permissions::parse_query(url.query().unwrap_or_default()).ok()
                != nyxid_permissions::parse_query(self.query.as_deref().unwrap_or_default()).ok()
        {
            return Err(policy::denied());
        }
        if crate::services::mcp_service::http_carries_method_override(
            request.method().as_str(),
            headers,
            url.query(),
            body,
        ) {
            return Err(policy::denied());
        }
        let final_operation = google_sheets::classify(
            &self.catalog_slug,
            &self.connection_id,
            request.method(),
            &path,
            url.query(),
            body,
            headers
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
        )
        .ok_or_else(policy::denied)?;
        if final_operation.action != self.operation.action
            || final_operation.resource_id != self.operation.resource_id
        {
            return Err(policy::denied());
        }
        Ok(())
    }
}

pub(crate) async fn authorize_sheets_operation(
    db: &Database,
    turn: &GuestTurn,
    operation: &google_sheets::SheetsOperation,
) -> AppResult<()> {
    let policy = policy::load_bound_policy(db, turn)
        .await?
        .ok_or_else(policy::denied)?;
    policy::authorize(
        &policy,
        turn,
        &policy::GuestDataRequest {
            source_id: google_sheets::SOURCE_ID,
            collection_id: google_sheets::COLLECTION_ID,
            action: operation.action,
            record_id: Some(&operation.resource_id),
            record_owner_id: None,
            fields: &[],
        },
    )?;
    Ok(())
}

pub(crate) fn stateless_model_request(
    method: &reqwest::Method,
    path: &str,
    query: Option<&str>,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
) -> bool {
    *method == reqwest::Method::POST
        && query.is_none_or(str::is_empty)
        && !crate::services::mcp_service::http_carries_method_override(
            method.as_str(),
            headers,
            query,
            body,
        )
        && serde_json::from_slice(body).is_ok_and(|body| {
            crate::services::assistant_service_execution::stateless_inference(path, &body)
                // Provider-hosted tools do not pass through this bot's
                // data policy, and may publish parts of a private prompt.
                && body.get("tools").and_then(Value::as_array).is_none_or(|tools| {
                    !tools.iter().any(|tool| tool.get("type").and_then(Value::as_str) == Some("web_search"))
                })
                && ["input", "messages", "system"].into_iter().all(|field| {
                    body.get(field).is_none_or(text_model_content)
                })
        })
}

// A guest may send text and local tool exchanges. Media blocks and
// provider-side asset URLs can cause an independent fetch or disclosure.
fn text_model_content(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().all(text_model_content),
        Value::Object(fields) => {
            !fields
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "input_image"
                            | "image"
                            | "image_url"
                            | "input_audio"
                            | "audio"
                            | "input_file"
                            | "file"
                            | "document"
                            | "video"
                            | "video_url"
                    )
                })
                && fields.iter().all(|(name, item)| {
                    !matches!(
                        name.as_str(),
                        "file_url" | "image_url" | "audio_url" | "video_url"
                    ) && text_model_content(item)
                })
        }
        _ => true,
    }
}

pub(crate) async fn authorize_sheets(
    db: &Database,
    chat: &ChatAuthority,
    request: SheetsRequest<'_>,
) -> AppResult<AuthorizedSheets> {
    let catalog_slug = request.catalog_slug.ok_or_else(policy::denied)?;
    let connection_id = request.connection_id.ok_or_else(policy::denied)?;
    let turn = chat.channel_guest.as_ref().ok_or_else(policy::denied)?;
    if !chat.guest
        || !google_sheets::supports_destination(&request.target.base_url)
        || request.target.auth_method != "bearer"
        || request.target.catalog_slug.as_deref() != Some(catalog_slug)
        || request.target.service.identity_propagation_mode != "none"
        || request.target.service.forward_access_token
        || request.target.service.inject_delegation_token
        || !request.target.catalog_headers().is_empty()
        || !request.target.user_service_default_headers.is_empty()
    {
        return Err(policy::denied());
    }
    let operation = google_sheets::classify(
        catalog_slug,
        connection_id,
        request.method,
        request.path,
        request.query,
        request.body,
        request.content_type,
    )
    .ok_or_else(policy::denied)?;
    let authorized = AuthorizedSheets {
        action: operation.action,
        approval: None,
        operation,
        turn: turn.clone(),
        catalog_slug: catalog_slug.into(),
        connection_id: connection_id.into(),
        path: request.path.clone(),
        query: request.query.map(str::to_owned),
        body_hash: Sha256::digest(request.body).into(),
    };
    authorized.revalidate(db, chat).await?;
    Ok(authorized)
}

#[cfg(test)]
mod tests {
    use reqwest::{Client, Method};

    use super::*;
    use crate::models::channel_guest_access::{AnswerAudience, GuestActor, GuestAssurance};

    #[test]
    fn channel_guest_policy_blocks_provider_hosted_tools_and_stored_model_context() {
        let headers = reqwest::header::HeaderMap::new();
        let permits = |body: &[u8]| {
            stateless_model_request(&Method::POST, "/v1/responses", None, &headers, body)
        };
        assert!(permits(br#"{"model":"model","input":"question","store":false,"tools":[{"type":"function","name":"local_tool"}]}"#));
        assert!(!permits(br#"{"model":"model","input":"question","store":false,"tools":[{"type":"web_search"}]}"#));
        assert!(!permits(
            br#"{"model":"model","input":"question","store":true}"#
        ));
        assert!(!permits(
            br#"{"model":"model","store":false,"input":[{"role":"user","content":[{"type":"input_file","file_url":"https://example.test/private.pdf"}]}]}"#
        ));
        assert!(!permits(
            br#"{"model":"model","store":false,"input":[{"role":"user","content":[{"type":"input_image","image_url":"https://example.test/private.png"}]}]}"#
        ));
    }

    fn authorized(
        action: &'static str,
        resource: &str,
        url: &str,
        body: &[u8],
    ) -> AuthorizedSheets {
        let url = reqwest::Url::parse(url).unwrap();
        AuthorizedSheets {
            action,
            approval: None,
            operation: google_sheets::SheetsOperation {
                action,
                resource_id: resource.into(),
            },
            turn: GuestTurn {
                bot_id: "bot".into(),
                binding_id: "binding".into(),
                platform: "discord".into(),
                conversation_id: "room".into(),
                source_event_id: "event".into(),
                actor: GuestActor {
                    id: "guest-a".into(),
                    assurance: GuestAssurance::ProviderVerified,
                },
                audience: AnswerAudience::Shared,
                policy_revision: 3,
                activation_epoch: 2,
                bot_revision: 1,
                meta_grant_id: None,
            },
            catalog_slug: "api-google-sheets".into(),
            connection_id: "connection".into(),
            path: CanonicalPath::from_mcp_built(url.path()).unwrap(),
            query: url.query().map(str::to_owned),
            body_hash: Sha256::digest(body).into(),
        }
    }

    #[test]
    fn channel_guest_policy_final_http_request_cannot_change_read_scope() {
        let client = Client::new();
        let exact =
            "https://sheets.googleapis.com/v4/spreadsheets/event/values/Event_Config%21A1%3AH24";
        let permit = authorized("read", "connection/event/Event_Config!A1:H24", exact, &[]);
        assert!(
            permit
                .validate_final_request(&client.get(exact).build().unwrap())
                .is_ok()
        );
        for other in [
            exact.replace("/event/", "/other-workbook/"),
            exact.replace("H24", "H25"),
            exact.replace("Event_Config", "Questions"),
            exact.replace("sheets.googleapis.com", "drive.googleapis.com"),
            format!("{exact}?alt=media"),
        ] {
            assert!(
                permit
                    .validate_final_request(&client.get(other).build().unwrap())
                    .is_err()
            );
        }
        assert!(
            permit
                .validate_final_request(
                    &client
                        .get(exact)
                        .header("x-http-method-override", "DELETE")
                        .build()
                        .unwrap()
                )
                .is_err()
        );
    }

    #[test]
    fn channel_guest_policy_final_http_append_is_one_raw_row_to_an_explicit_tab() {
        let client = Client::new();
        let url = "https://sheets.googleapis.com/v4/spreadsheets/event/values/Questions%21A%3AH:append?valueInputOption=RAW&insertDataOption=INSERT_ROWS";
        let append = |url: &str, body: &'static str| {
            client
                .request(Method::POST, url)
                .header("content-type", "application/json")
                .body(body)
                .build()
                .unwrap()
        };
        let valid_body = r#"{"values":[["Question for the host"]]}"#;
        let permit = authorized(
            "create",
            "connection/event/Questions",
            url,
            valid_body.as_bytes(),
        );
        assert!(
            permit
                .validate_final_request(&append(url, valid_body))
                .is_ok()
        );
        assert!(
            permit
                .validate_final_request(&append(
                    &url.replace("Questions", "Internal_Notes"),
                    valid_body,
                ))
                .is_err()
        );
        assert!(
            permit
                .validate_final_request(&append(&url.replace("RAW", "USER_ENTERED"), valid_body,))
                .is_err()
        );
        assert!(
            permit
                .validate_final_request(&append(
                    &format!("{url}&includeValuesInResponse=true"),
                    valid_body,
                ))
                .is_err()
        );
        assert!(
            permit
                .validate_final_request(&append(url, r#"{"values":[["First"],["Second"]]}"#,))
                .is_err()
        );
    }
}
