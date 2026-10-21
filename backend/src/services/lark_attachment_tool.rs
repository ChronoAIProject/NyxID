//! Trusted, stored-catalog attachment operations. Instance specs never grant
//! access to conversation bytes, even when they mount an exact hosted overlay.
use super::{catalog_spec_registry, catalog_spec_sync, mcp_service::McpToolEndpoint};
use crate::{
    errors::{AppError, AppResult},
    models::downstream_service::DownstreamService,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const UPLOAD_OPERATION: &str = "bitable_media_upload";
pub const UPLOAD_PATH: &str = "/drive/v1/medias/upload_all";
pub const COMPOSITE_OPERATION: &str = "bitable_record_create_with_attachments";
pub const COMPOSITE_PATH: &str =
    "/nyxid/managed/bitable/apps/{app_token}/tables/{table_id}/records";
pub const FIELDS_OPERATION: &str = "bitable_fields_list";
pub const CREATE_OPERATION: &str = "bitable_record_create";
const MARK: &str = "x-nyxid-attachment-input";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentInput {
    pub field: String,
    pub size_field: String,
    pub filename_field: String,
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "provenance", content = "contract", rename_all = "snake_case")]
pub enum Projection {
    Stored(AttachmentInput),
    Managed(ManagedOperation),
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedOperation {
    pub kind: String,
    pub version: u32,
    pub fields_operation: String,
    pub upload_operation: String,
    pub create_operation: String,
    #[serde(default)]
    pub contract_digests: std::collections::BTreeMap<String, String>,
}

pub fn refused() -> AppError {
    AppError::Forbidden(
        "Attachment uploads require a trusted catalog operation in a live owner chat".into(),
    )
}

/// The managed route is an internal invocation identity, never a provider path.
pub(crate) fn refuse_reserved_path(path: &str) -> AppResult<()> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let path = path.strip_prefix("open-apis/").unwrap_or(path);
    if path.starts_with("nyxid/managed/bitable/") {
        return Err(refused());
    }
    Ok(())
}

pub fn is_attachment_operation(endpoint: &McpToolEndpoint) -> bool {
    endpoint.attachment_input.is_some()
        || endpoint.name == UPLOAD_OPERATION
        || endpoint.path == UPLOAD_PATH
        || endpoint.path == format!("/open-apis{UPLOAD_PATH}")
        || endpoint.name == COMPOSITE_OPERATION
        || endpoint.path == COMPOSITE_PATH
        || endpoint.path == format!("/open-apis{COMPOSITE_PATH}")
}

/// Name/route matching only identifies operations that must refuse ordinary
/// dispatch. It never grants a materializer or attachment access.
pub fn untrusted(endpoint: &McpToolEndpoint) -> Option<Projection> {
    is_attachment_operation(endpoint).then_some(Projection::Refused)
}

fn inputs(endpoint: &McpToolEndpoint) -> Value {
    json!({
        "method":endpoint.method.to_ascii_uppercase(), "path":endpoint.path,
        "target":endpoint.target_id, "parameters":endpoint.parameters,
        "content_type":endpoint.request_content_type.as_deref().map(super::content_type::normalize_content_type),
        "schema":endpoint.request_body_schema,"required":endpoint.request_body_required,
        "async":endpoint.async_operation,
    })
}

fn validate_mark(mark: &Value, schema: &Value) -> AppResult<AttachmentInput> {
    let contract: AttachmentInput = serde_json::from_value(mark.clone()).map_err(|_| refused())?;
    if contract.version != 1
        || contract.field != "file"
        || contract.size_field != "size"
        || contract.filename_field != "file_name"
        || schema["type"] != "object"
        || schema["additionalProperties"] != false
        || schema["properties"][&contract.field]["type"] != "string"
        || schema["properties"][&contract.size_field]["type"] != "integer"
        || schema["properties"][&contract.filename_field]["type"] != "string"
        || schema["required"]
            != json!([
                "file_name",
                "parent_type",
                "parent_node",
                "size",
                "extra",
                "file"
            ])
    {
        return Err(refused());
    }
    Ok(contract)
}

/// Reconstruct authority from the compiled overlay and resolved SYSTEM catalog
/// identity. The sync parser supplies the same normalized target/origin selector
/// as it persists; no caller-supplied mark or general catalog flag is trusted.
pub fn stored_projection(
    catalog: Option<&DownstreamService>,
    endpoint: &McpToolEndpoint,
) -> Option<Projection> {
    let candidate = untrusted(endpoint)?;
    let trusted = (|| {
        let service = catalog?;
        if service.created_by != "system"
            || service.tools_manager_created
            || !matches!(
                catalog_spec_registry::spec_key_for_slug(&service.slug),
                Some("lark" | "lark-bot")
            )
        {
            return None;
        }
        let spec = catalog_spec_registry::spec_for_slug(&service.slug)?;
        let compiled = catalog_spec_sync::destination_endpoint_inputs(service, &spec)
            .ok()?
            .into_iter()
            .find(|input| input.name == endpoint.name)?;
        let expected = McpToolEndpoint {
            async_operation: compiled.async_operation,
            target_id: compiled.target_id,
            name: compiled.name,
            method: compiled.method,
            path: compiled.path,
            parameters: compiled.parameters,
            request_body_schema: compiled.request_body_schema,
            request_content_type: compiled.request_content_type,
            request_body_required: compiled.request_body_required,
            ..Default::default()
        };
        if inputs(endpoint) != inputs(&expected) {
            return None;
        }
        let mark = &spec["paths"][UPLOAD_PATH][expected.method.to_ascii_lowercase()][MARK];
        let contract = validate_mark(mark, expected.request_body_schema.as_ref()?).ok()?;
        Some(Projection::Stored(contract))
    })();
    Some(trusted.unwrap_or(candidate))
}

/// Composite authority requires the complete stored dependency set. A caller
/// loading just the reserved endpoint retains the refusal projection.
pub fn stored_projections(catalog: Option<&DownstreamService>, endpoints: &mut [McpToolEndpoint]) {
    for endpoint in endpoints.iter_mut() {
        endpoint.attachment_input = stored_projection(catalog, endpoint);
    }
    let managed = (|| {
        let catalog = catalog.filter(|catalog| {
            catalog.created_by == "system"
                && !catalog.tools_manager_created
                && matches!(
                    catalog_spec_registry::spec_key_for_slug(&catalog.slug),
                    Some("lark" | "lark-bot")
                )
        })?;
        let spec = catalog_spec_registry::spec_for_slug(&catalog.slug)?;
        let compiled = catalog_spec_sync::destination_endpoint_inputs(catalog, &spec).ok()?;
        let mut mark: ManagedOperation = serde_json::from_value(
            spec["paths"][COMPOSITE_PATH]["post"]["x-nyxid-managed-operation"].clone(),
        )
        .ok()?;
        if mark.kind != "lark_bitable_record_with_attachments"
            || mark.version != 1
            || mark.fields_operation != FIELDS_OPERATION
            || mark.upload_operation != UPLOAD_OPERATION
            || mark.create_operation != CREATE_OPERATION
            || !mark.contract_digests.is_empty()
        {
            return None;
        }
        for name in [
            COMPOSITE_OPERATION,
            FIELDS_OPERATION,
            UPLOAD_OPERATION,
            CREATE_OPERATION,
        ] {
            let mut matches = endpoints.iter().filter(|endpoint| endpoint.name == name);
            let actual = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            let expected = compiled.iter().find(|endpoint| endpoint.name == name)?;
            let expected = McpToolEndpoint {
                name: expected.name.clone(),
                method: expected.method.clone(),
                path: expected.path.clone(),
                target_id: expected.target_id.clone(),
                parameters: expected.parameters.clone(),
                request_body_schema: expected.request_body_schema.clone(),
                request_content_type: expected.request_content_type.clone(),
                request_body_required: expected.request_body_required,
                async_operation: expected.async_operation.clone(),
                ..Default::default()
            };
            if inputs(actual) != inputs(&expected) {
                return None;
            }
            if name != COMPOSITE_OPERATION {
                mark.contract_digests.insert(
                    name.into(),
                    super::mcp_service::endpoint_contract_digest(actual),
                );
            }
        }
        Some(mark)
    })();
    if let Some(managed) = managed {
        for endpoint in endpoints {
            if endpoint.name == COMPOSITE_OPERATION {
                endpoint.attachment_input = Some(Projection::Managed(managed.clone()));
            }
        }
    }
}

pub(super) mod multipart;
use super::assistant_acknowledgement_service::ChatAuthority;
use super::assistant_upload_service::{self as uploads, byte_budget};
use super::mcp_service::{McpToolService, McpToolSource};
use mongodb::{Database, bson::doc};

pub struct Binding {
    chat: ChatAuthority,
    attachment_id: String,
    _reservation: byte_budget::ByteReservation,
}
pub struct Materialized {
    pub body: bytes::Bytes,
    pub content_type: String,
    pub binding: Binding,
}

pub async fn writes_enabled(db: &Database, chat: Option<&ChatAuthority>) -> AppResult<bool> {
    let Some(chat) = chat.filter(|chat| !chat.guest) else {
        return Ok(false);
    };
    super::feature_flag_service::personal_flag_enabled(
        db,
        &chat.user_id,
        super::feature_flag_service::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
    )
    .await
}

/// Discovery and execution share the same projection; an invalid candidate is
/// omitted rather than being advertised as an ordinary HTTP upload operation.
pub async fn filter_discovery(
    db: &Database,
    chat: Option<&ChatAuthority>,
    services: &mut [McpToolService],
) -> AppResult<()> {
    if !services
        .iter()
        .flat_map(|service| &service.endpoints)
        .any(is_attachment_operation)
    {
        return Ok(());
    }
    let enabled = writes_enabled(db, chat).await?;
    for service in services {
        service.endpoints.retain(|endpoint| {
            !is_attachment_operation(endpoint)
                || (enabled
                    && matches!(
                        endpoint.attachment_input,
                        Some(Projection::Stored(_) | Projection::Managed(_))
                    ))
        });
    }
    Ok(())
}

pub async fn materialize(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    chat: Option<&ChatAuthority>,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
    args: &Value,
) -> AppResult<Materialized> {
    if !matches!(endpoint.attachment_input, Some(Projection::Stored(_)))
        || endpoint.async_operation.is_some()
        || !writes_enabled(db, chat).await?
    {
        return Err(refused());
    }
    let chat = chat.ok_or_else(refused)?;
    validate_contract(db, service, endpoint).await?;
    let id = args["attachment_id"].as_str().ok_or_else(refused)?;
    let attachment = uploads::for_egress(db, chat, id).await?;
    let size = usize::try_from(attachment.size())
        .ok()
        .filter(|n| *n > 0 && *n <= super::attachment_extraction::MAX_BYTES)
        .ok_or_else(refused)?;
    let fields = multipart::fields(args, attachment.label(), size)?;
    let reservation = byte_budget::reserve(size).await?;
    let bytes = uploads::egress_bytes(db, keys, chat, &attachment).await?;
    let (body, content_type) = multipart::encode(&fields, attachment.content_type(), &bytes)?;
    Ok(Materialized {
        body,
        content_type,
        binding: Binding {
            chat: chat.clone(),
            attachment_id: id.into(),
            _reservation: reservation,
        },
    })
}

pub fn binding_matches(binding: &Binding, user: &str, key: Option<&str>) -> bool {
    binding.chat.user_id == user && key == Some(binding.chat.api_key_id.as_str())
}

pub async fn validate_binding(
    db: &Database,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
    binding: &Binding,
) -> AppResult<()> {
    if !writes_enabled(db, Some(&binding.chat)).await? {
        return Err(refused());
    }
    validate_contract(db, service, endpoint).await?;
    uploads::for_egress(db, &binding.chat, &binding.attachment_id).await?;
    Ok(())
}

async fn validate_contract(
    db: &Database,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
) -> AppResult<()> {
    use crate::models::{downstream_service, service_endpoint, user_endpoint, user_service};
    let catalog_id = match &service.source {
        McpToolSource::UserManaged {
            catalog_service_id: Some(id),
            ..
        } => id,
        McpToolSource::Platform {
            downstream_service_id,
        } => downstream_service_id,
        _ => return Err(refused()),
    };
    let catalog = db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .find_one(doc! {"_id":catalog_id,"is_active":true})
        .await?
        .ok_or_else(refused)?;
    if let McpToolSource::UserManaged {
        user_service_id, ..
    } = &service.source
    {
        let instance = super::service_history::collection::<user_service::UserService>(
            db,
            user_service::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":user_service_id,"is_active":true,"catalog_service_id":catalog_id})
        .await?
        .ok_or_else(refused)?;
        let mounted = super::service_history::collection::<user_endpoint::UserEndpoint>(
            db,
            user_endpoint::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&instance.endpoint_id})
        .await?
        .ok_or_else(refused)?;
        if mounted.openapi_spec_url.is_some() {
            return Err(refused());
        }
    }
    let row = db
        .collection::<service_endpoint::ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
        .find_one(doc! {"_id":&endpoint.endpoint_id,"service_id":catalog_id,"is_active":true})
        .await?
        .ok_or_else(refused)?;
    if row.publication != service_endpoint::PublicationState::Published {
        return Err(refused());
    }
    let mut loaded = super::mcp_service::service_endpoints_to_mcp(&[&row])
        .pop()
        .ok_or_else(refused)?;
    loaded.attachment_input = stored_projection(Some(&catalog), &loaded);
    if !matches!(loaded.attachment_input, Some(Projection::Stored(_)))
        || super::mcp_service::endpoint_contract_digest(&loaded)
            != super::mcp_service::endpoint_contract_digest(endpoint)
    {
        return Err(refused());
    }
    Ok(())
}

pub fn input_schema(endpoint: &McpToolEndpoint) -> Value {
    let mut schema = endpoint
        .request_body_schema
        .clone()
        .unwrap_or_else(|| json!({}));
    let properties = schema["properties"]
        .as_object_mut()
        .expect("trusted object schema");
    properties.remove("file");
    properties.insert("attachment_id".into(),json!({"type":"string","description":"Attachment ID from this live owner conversation; never a URL or file path."}));
    schema["required"] = json!(["attachment_id", "parent_type", "parent_node"]);
    schema
}

pub fn validate_response(status: u16, text: &str) -> AppResult<()> {
    let body: Value = serde_json::from_str(text).map_err(|_| {
        AppError::BadRequest("Attachment upload returned an invalid response".into())
    })?;
    if !(200..300).contains(&status)
        || body["code"].as_i64() != Some(0)
        || body["data"]["file_token"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        return Err(AppError::BadRequest(
            "Attachment upload failed; no usable file_token was returned".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::mcp_service::{endpoint_contract_digest, prepare_proxy_tool_call};

    fn service() -> super::super::mcp_service::McpToolService {
        super::super::mcp_service::McpToolService {
            workspace_destinations_pending: false,
            service_id: "catalog".into(),
            service_name: "Lark".into(),
            service_slug: "api-lark".into(),
            description: None,
            service_category: "connection".into(),
            endpoints: Vec::new(),
            durable_endpoint_metadata: Default::default(),
            source: super::super::mcp_service::McpToolSource::Platform {
                downstream_service_id: "catalog".into(),
            },
            executable: true,
            is_generic_proxy: false,
            invalid_openapi_contract: false,
            recommended_skills: Vec::new(),
            recommended_skill_refs: None,
            skills_revision: None,
            proxy_operation_policy: None,
        }
    }

    fn compiled(slug: &str) -> (DownstreamService, McpToolEndpoint) {
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.slug = slug.into();
        catalog.created_by = "system".into();
        let spec = catalog_spec_registry::spec_for_slug(slug).unwrap();
        let input = catalog_spec_sync::destination_endpoint_inputs(&catalog, &spec)
            .unwrap()
            .into_iter()
            .find(|input| input.name == UPLOAD_OPERATION)
            .unwrap();
        let endpoint = McpToolEndpoint {
            endpoint_id: "upload".into(),
            name: input.name,
            method: input.method,
            path: input.path,
            target_id: input.target_id,
            parameters: input.parameters,
            request_body_schema: input.request_body_schema,
            request_content_type: input.request_content_type,
            request_body_required: input.request_body_required,
            response: input.response,
            ..Default::default()
        };
        (catalog, endpoint)
    }

    fn composite_endpoints(slug: &str) -> (DownstreamService, Vec<McpToolEndpoint>) {
        let (catalog, _) = compiled(slug);
        let endpoints = catalog_spec_sync::destination_endpoint_inputs(
            &catalog,
            &catalog_spec_registry::spec_for_slug(slug).unwrap(),
        )
        .unwrap()
        .into_iter()
        .map(|input| McpToolEndpoint {
            endpoint_id: input.name.clone(),
            name: input.name,
            method: input.method,
            path: input.path,
            target_id: input.target_id,
            parameters: input.parameters,
            request_body_schema: input.request_body_schema,
            request_content_type: input.request_content_type,
            request_body_required: input.request_body_required,
            response: input.response,
            async_operation: input.async_operation,
            ..Default::default()
        })
        .collect();
        (catalog, endpoints)
    }

    #[test]
    fn lark_base_write_projection_binds_every_dependency_and_rejects_missing_or_edited_inputs() {
        for slug in ["api-lark", "api-lark-bot", "api-feishu", "api-feishu-bot"] {
            let (catalog, raw) = composite_endpoints(slug);
            let mut endpoints = raw.clone();
            stored_projections(Some(&catalog), &mut endpoints);
            let composite = endpoints
                .iter()
                .find(|ep| ep.name == COMPOSITE_OPERATION)
                .unwrap();
            let Some(Projection::Managed(mark)) = &composite.attachment_input else {
                panic!("missing trusted composite")
            };
            assert_eq!(mark.contract_digests.len(), 3);
            assert!(prepare_proxy_tool_call(&service(), composite, &json!({})).is_err());
            for name in [
                COMPOSITE_OPERATION,
                FIELDS_OPERATION,
                UPLOAD_OPERATION,
                CREATE_OPERATION,
            ] {
                let mut edited = raw.clone();
                edited
                    .iter_mut()
                    .find(|ep| ep.name == name)
                    .unwrap()
                    .request_body_required ^= true;
                stored_projections(Some(&catalog), &mut edited);
                assert_eq!(
                    edited
                        .iter()
                        .find(|ep| ep.name == COMPOSITE_OPERATION)
                        .unwrap()
                        .attachment_input,
                    Some(Projection::Refused)
                );
                if name != COMPOSITE_OPERATION {
                    let mut missing = raw.clone();
                    missing.retain(|ep| ep.name != name);
                    stored_projections(Some(&catalog), &mut missing);
                    assert_eq!(
                        missing
                            .iter()
                            .find(|ep| ep.name == COMPOSITE_OPERATION)
                            .unwrap()
                            .attachment_input,
                        Some(Projection::Refused)
                    );
                }
            }
            let mut untrusted = raw;
            stored_projections(None, &mut untrusted);
            assert_eq!(
                untrusted
                    .iter()
                    .find(|ep| ep.name == COMPOSITE_OPERATION)
                    .unwrap()
                    .attachment_input,
                Some(Projection::Refused)
            );
        }
    }

    #[test]
    fn lark_attachment_projection_requires_system_identity_and_all_execution_inputs() {
        for slug in ["api-lark", "api-lark-bot", "api-feishu", "api-feishu-bot"] {
            let (catalog, endpoint) = compiled(slug);
            assert!(matches!(
                stored_projection(Some(&catalog), &endpoint),
                Some(Projection::Stored(_))
            ));
            let mut custom = catalog.clone();
            custom.created_by = "user".into();
            assert_eq!(
                stored_projection(Some(&custom), &endpoint),
                Some(Projection::Refused)
            );
            custom.created_by = "system".into();
            custom.slug = "custom-lark".into();
            assert_eq!(
                stored_projection(Some(&custom), &endpoint),
                Some(Projection::Refused)
            );
            assert_eq!(
                stored_projection(None, &endpoint),
                Some(Projection::Refused)
            );
            let mut variants = Vec::new();
            let mut edited = endpoint.clone();
            edited.path = "/other".into();
            variants.push(edited);
            let mut edited = endpoint.clone();
            edited.method = "PUT".into();
            variants.push(edited);
            let mut edited = endpoint.clone();
            edited.parameters = Some(
                json!([{"name":"evil","in":"query","required":true,"schema":{"type":"string"}}]),
            );
            variants.push(edited);
            let mut edited = endpoint.clone();
            edited.request_body_schema.as_mut().unwrap()["properties"]["size"]["type"] =
                json!("string");
            variants.push(edited);
            let mut edited = endpoint.clone();
            edited.request_content_type = Some("application/json".into());
            variants.push(edited);
            let mut edited = endpoint.clone();
            edited.request_body_required = false;
            variants.push(edited);
            let mut edited = endpoint.clone();
            edited.target_id = Some("foreign-origin".into());
            variants.push(edited);
            let mut edited = endpoint.clone();
            edited.name = "other".into();
            variants.push(edited);
            for mut edited in variants {
                edited.attachment_input = stored_projection(Some(&catalog), &edited);
                assert_eq!(edited.attachment_input, Some(Projection::Refused));
                assert!(prepare_proxy_tool_call(&service(), &edited, &json!({})).is_err());
            }
        }
    }

    #[test]
    fn lark_attachment_projection_is_versioned_and_absent_contract_digest_is_unchanged() {
        let (catalog, mut endpoint) = compiled("api-lark");
        let old = endpoint_contract_digest(&endpoint);
        let legacy = json!({"contract_version":"nyxid-exact-endpoint.v1","endpoint_id":endpoint.endpoint_id,
            "method":endpoint.method,"path":endpoint.path,"parameters":endpoint.parameters,
            "request_body_schema":endpoint.request_body_schema,"request_content_type":endpoint.request_content_type,
            "request_body_required":endpoint.request_body_required,"response":endpoint.response});
        assert_eq!(old, super::super::mcp_service::canonical_sha256(legacy));
        endpoint.attachment_input = stored_projection(Some(&catalog), &endpoint);
        assert_ne!(old, endpoint_contract_digest(&endpoint));
        let before = endpoint_contract_digest(&endpoint);
        if let Some(Projection::Stored(contract)) = &mut endpoint.attachment_input {
            contract.version += 1;
        }
        assert_ne!(before, endpoint_contract_digest(&endpoint));
        let schema = endpoint.request_body_schema.as_ref().unwrap();
        assert!(validate_mark(&json!({"field":"size","size_field":"size","filename_field":"file_name","version":1}),schema).is_err());
        assert!(validate_mark(&json!({"field":"file","size_field":"size","filename_field":"file_name","version":2}),schema).is_err());
        assert!(schema.get(MARK).is_none());
    }
}
