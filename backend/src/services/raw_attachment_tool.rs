//! Chat-only input schemas and materialization for published binary catalog operations.
//! No endpoint projection or durable contract is changed.
use super::{
    assistant_acknowledgement_service::ChatAuthority,
    assistant_upload_service as uploads,
    lark_attachment_tool::{self as attachments, Binding, Materialized},
    mcp_service::{self as mcp, McpToolEndpoint, McpToolService, McpToolSource, RequestBodyMode},
};
use crate::{crypto::aes::EncryptionKeys, errors::AppResult};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

/// Only an eligible chat caller can interpret an otherwise undeclared argument
/// as a server-side file handle. Every other call keeps the original parser.
pub(crate) fn has_handle(endpoint: &McpToolEndpoint, args: &Value) -> bool {
    args.get("attachment_id").is_some() && !declares_handle(endpoint)
}

fn declares_handle(endpoint: &McpToolEndpoint) -> bool {
    endpoint
        .parameters
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|params| params.iter().any(|param| param["name"] == "attachment_id"))
        || endpoint
            .request_body_schema
            .as_ref()
            .is_some_and(schema_declares_handle)
}

// Conservatively preserve provider-owned fields in composed/local-reference
// schemas too. A collision anywhere in a body schema disables this optional
// capability; it must never reinterpret an existing provider argument.
fn schema_declares_handle(schema: &Value) -> bool {
    match schema {
        Value::Object(object) => {
            object
                .get("properties")
                .is_some_and(|properties| properties.get("attachment_id").is_some())
                || object.values().any(schema_declares_handle)
        }
        Value::Array(values) => values.iter().any(schema_declares_handle),
        _ => false,
    }
}

/// Proof of the single pre-materialization capability check. The references bind
/// it to this caller, endpoint and argument set; callers cannot construct it.
pub(crate) struct ValidatedHandle<'a> {
    chat: &'a ChatAuthority,
    endpoint: &'a McpToolEndpoint,
    args: &'a Value,
}

pub(crate) async fn uses_handle<'a>(
    db: &Database,
    chat: Option<&'a ChatAuthority>,
    service: &McpToolService,
    endpoint: &'a McpToolEndpoint,
    args: &'a Value,
) -> AppResult<Option<ValidatedHandle<'a>>> {
    if !has_handle(endpoint, args) || !candidate(service, endpoint) {
        return Ok(None);
    }
    let Some(chat) = super::assistant_tool_files::eligible(chat) else {
        return Ok(None);
    };
    if args.get(mcp::request_body_field_name(endpoint)).is_some() {
        return Err(crate::errors::AppError::BadRequest(
            "Supply exactly one of the base64 body or attachment_id for a file upload".into(),
        ));
    }
    if let Err(error) = validate_contract(db, service, endpoint).await {
        tracing::warn!(
            code = error.error_code(),
            "File upload contract unavailable"
        );
        return Err(crate::errors::AppError::BadRequest(
            "File uploads by attachment_id are unavailable for this operation; it must be a published catalog operation without a mounted spec".into(),
        ));
    }
    Ok(Some(ValidatedHandle {
        chat,
        endpoint,
        args,
    }))
}

pub(crate) fn candidate(service: &McpToolService, endpoint: &McpToolEndpoint) -> bool {
    !service.is_generic_proxy
        && !attachments::is_attachment_operation(endpoint)
        && endpoint.async_operation.is_none()
        && Uuid::parse_str(&endpoint.endpoint_id).is_ok()
        && mcp::request_body_mode(endpoint) == RequestBodyMode::Binary
        && !declares_handle(endpoint)
        && matches!(
            &service.source,
            McpToolSource::Platform { .. }
                | McpToolSource::UserManaged {
                    catalog_service_id: Some(_),
                    node_id: None,
                    ..
                }
        )
}

/// Used again at egress, including failed mounted-spec fallback to catalog rows.
pub(crate) async fn validate_contract(
    db: &Database,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
) -> AppResult<()> {
    use crate::models::{
        downstream_service as catalog, service_endpoint as endpoints, user_endpoint, user_service,
    };
    if !candidate(service, endpoint) {
        return Err(attachments::refused());
    }
    let catalog_id = match &service.source {
        McpToolSource::Platform {
            downstream_service_id,
        } => downstream_service_id,
        McpToolSource::UserManaged {
            catalog_service_id: Some(id),
            ..
        } => id,
        _ => return Err(attachments::refused()),
    };
    db.collection::<catalog::DownstreamService>(catalog::COLLECTION_NAME)
        .find_one(doc! {"_id":catalog_id,"is_active":true})
        .await?
        .ok_or_else(attachments::refused)?;
    if let McpToolSource::UserManaged {
        user_service_id, ..
    } = &service.source
    {
        let row = super::service_history::collection::<user_service::UserService>(db,user_service::COLLECTION_NAME)
            .find_one(doc!{"_id":user_service_id,"catalog_service_id":catalog_id,"is_active":true,"node_id":bson::Bson::Null})
            .await?.ok_or_else(attachments::refused)?;
        let mounted = super::service_history::collection::<user_endpoint::UserEndpoint>(
            db,
            user_endpoint::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&row.endpoint_id})
        .await?
        .ok_or_else(attachments::refused)?;
        if mounted.openapi_spec_url.is_some() {
            return Err(attachments::refused());
        }
    }
    let row = db
        .collection::<endpoints::ServiceEndpoint>(endpoints::COLLECTION_NAME)
        .find_one(doc! {"_id":&endpoint.endpoint_id,"service_id":catalog_id,"is_active":true})
        .await?
        .ok_or_else(attachments::refused)?;
    let loaded = mcp::service_endpoints_to_mcp(&[&row])
        .pop()
        .ok_or_else(attachments::refused)?;
    if row.publication != endpoints::PublicationState::Published
        || row.execution != endpoints::ExecutionKind::HttpOperation
        || mcp::endpoint_contract_digest(&loaded) != mcp::endpoint_contract_digest(endpoint)
    {
        return Err(attachments::refused());
    }
    Ok(())
}

fn catalog_id(service: &McpToolService) -> Option<&str> {
    match &service.source {
        McpToolSource::Platform {
            downstream_service_id,
        } => Some(downstream_service_id),
        McpToolSource::UserManaged {
            catalog_service_id, ..
        } => catalog_service_id.as_deref(),
        _ => None,
    }
}

pub(crate) fn input_schema(endpoint: &McpToolEndpoint) -> Value {
    let mut schema = mcp::build_input_schema(endpoint);
    let body = mcp::request_body_field_name(endpoint);
    let description = schema["properties"][&body]["description"]
        .as_str()
        .unwrap_or_default();
    schema["properties"][&body]["description"] = json!(format!(
        "{description}. Use for content you wrote yourself; supply exactly one of this field or attachment_id. For an existing uploaded or downloaded file, use attachment_id instead of reconstructing its bytes."
    ));
    schema["properties"]["attachment_id"] = json!({"type":"string","format":"uuid",
        "description":"File attachment ID from this owner conversation (tool files only in their producing turn). Use for an existing uploaded or downloaded file; NyxID supplies the exact bytes. Supply exactly one of attachment_id or the base64 body. Supply declared parameters such as fileName yourself; never reconstruct file bytes or supply URLs or paths."});
    let mut required = schema["required"].as_array().cloned().unwrap_or_default();
    required.retain(|name| name != &body);
    schema["required"] = json!(required);
    schema["oneOf"] = json!([{"required":[body]}, {"required":["attachment_id"]}]);
    schema
}

/// Schema overrides are applied to the rendered MCP definitions only. Config,
/// search contracts, exact descriptors and stored endpoints keep base64 bodies.
pub(crate) async fn discovery_schemas(
    db: &Database,
    chat: Option<&ChatAuthority>,
    services: &[McpToolService],
) -> AppResult<HashMap<String, Value>> {
    match lookup_schemas(db, chat, services).await {
        Ok(schemas) => Ok(schemas),
        Err(error) => {
            tracing::warn!(
                code = error.error_code(),
                "File upload schemas unavailable; using original schemas"
            );
            Ok(HashMap::new())
        }
    }
}

async fn lookup_schemas(
    db: &Database,
    chat: Option<&ChatAuthority>,
    services: &[McpToolService],
) -> AppResult<HashMap<String, Value>> {
    let mut schemas = HashMap::new();
    if !services.iter().any(|service| {
        service
            .endpoints
            .iter()
            .any(|endpoint| candidate(service, endpoint))
    }) || super::assistant_tool_files::eligible(chat).is_none()
    {
        return Ok(schemas);
    }
    use crate::models::{
        downstream_service as catalog, service_endpoint as endpoints, user_endpoint, user_service,
    };
    let candidates: Vec<_> = services
        .iter()
        .flat_map(|service| {
            service
                .endpoints
                .iter()
                .filter(|endpoint| candidate(service, endpoint))
                .map(move |endpoint| (service, endpoint))
        })
        .collect();
    let ids: Vec<_> = candidates
        .iter()
        .map(|(_, endpoint)| &endpoint.endpoint_id)
        .collect();
    // One endpoint query for the entire discovery request, independent of tool count.
    let rows: Vec<endpoints::ServiceEndpoint> = db
        .collection(endpoints::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":ids},"is_active":true})
        .await?
        .try_collect()
        .await?;
    let catalog_ids: Vec<_> = candidates
        .iter()
        .filter_map(|(service, _)| catalog_id(service))
        .collect();
    let catalogs: Vec<bson::Document> = db
        .collection(catalog::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":catalog_ids},"is_active":true})
        .projection(doc! {"_id":1})
        .await?
        .try_collect()
        .await?;
    let instance_ids: Vec<_> = candidates
        .iter()
        .filter_map(|(service, _)| match &service.source {
            McpToolSource::UserManaged {
                user_service_id, ..
            } => Some(user_service_id),
            _ => None,
        })
        .collect();
    let instances: Vec<user_service::UserService> = if instance_ids.is_empty() {
        Vec::new()
    } else {
        super::service_history::collection(db, user_service::COLLECTION_NAME)
            .find(doc! {"_id":{"$in":instance_ids},"is_active":true,"node_id":bson::Bson::Null})
            .await?
            .try_collect()
            .await?
    };
    let mounted_ids: Vec<_> = instances
        .iter()
        .map(|instance| &instance.endpoint_id)
        .collect();
    let mounts: Vec<user_endpoint::UserEndpoint> = if mounted_ids.is_empty() {
        Vec::new()
    } else {
        super::service_history::collection(db, user_endpoint::COLLECTION_NAME)
            .find(doc! {"_id":{"$in":mounted_ids},"openapi_spec_url":bson::Bson::Null})
            .await?
            .try_collect()
            .await?
    };
    for (service, endpoint) in candidates {
        let Some(catalog) = catalog_id(service) else {
            continue;
        };
        if !catalogs
            .iter()
            .any(|row| row.get_str("_id").ok() == Some(catalog))
        {
            continue;
        }
        if let McpToolSource::UserManaged {
            user_service_id, ..
        } = &service.source
            && !instances.iter().any(|instance| {
                &instance.id == user_service_id
                    && instance.catalog_service_id.as_deref() == Some(catalog)
                    && mounts.iter().any(|mount| mount.id == instance.endpoint_id)
            })
        {
            continue;
        }
        if let Some(row) = rows.iter().find(|row| {
            row.id == endpoint.endpoint_id
                && row.service_id == catalog
                && row.publication == endpoints::PublicationState::Published
                && row.execution == endpoints::ExecutionKind::HttpOperation
        }) && let Some(loaded) = mcp::service_endpoints_to_mcp(&[row]).pop()
            && mcp::endpoint_contract_digest(&loaded) == mcp::endpoint_contract_digest(endpoint)
        {
            schemas.insert(
                format!("{}__{}", service.service_slug, endpoint.name),
                input_schema(endpoint),
            );
        }
    }
    Ok(schemas)
}

pub(crate) async fn materialize(
    db: &Database,
    keys: &EncryptionKeys,
    validated: ValidatedHandle<'_>,
) -> AppResult<Materialized> {
    let ValidatedHandle {
        chat,
        endpoint,
        args,
    } = validated;
    let id = args["attachment_id"]
        .as_str()
        .ok_or_else(attachments::refused)?;
    let attachment = uploads::for_file_egress(db, chat, id).await?;
    let size = usize::try_from(attachment.size())
        .ok()
        .filter(|n| *n > 0 && *n <= super::attachment_extraction::MAX_BYTES)
        .ok_or_else(attachments::refused)?;
    let reservation = uploads::byte_budget::reserve(size).await?;
    let snapshot = metadata_identity(db, id).await?;
    let bytes =
        zeroize::Zeroizing::new(uploads::file_egress_bytes(db, keys, chat, &attachment).await?);
    if snapshot != metadata_identity(db, id).await? {
        return Err(attachments::refused());
    }
    let content_type = upload_content_type(endpoint, attachment.content_type())?;
    let parameters = args.as_object().map(|args| {
        args.iter()
            .filter(|(key, _)| {
                key.as_str() != "attachment_id" && key.as_str() != "acknowledgement_id"
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    });
    let display = json!({"attachment_id":id,"name":attachment.label(),"size":size,"sha256":hex::encode(Sha256::digest(&bytes)),"type":content_type,"parameters":parameters});
    Ok(Materialized {
        body: bytes::Bytes::from_owner(bytes),
        content_type,
        binding: Binding::raw(chat.clone(), id.into(), reservation, snapshot, display),
    })
}

fn upload_content_type(endpoint: &McpToolEndpoint, attachment_type: &str) -> AppResult<String> {
    let declared = endpoint
        .request_content_type
        .as_deref()
        .unwrap_or("application/octet-stream");
    let declared_normalized = super::content_type::normalize_content_type(declared);
    let mime = if declared_normalized.contains('*') {
        let attachment_type = super::content_type::normalize_content_type(attachment_type);
        if declared_normalized != "*/*"
            && !attachment_type.starts_with(declared_normalized.trim_end_matches('*'))
        {
            return Err(attachments::refused());
        }
        attachment_type
    } else {
        declared.to_owned()
    };
    reqwest::header::HeaderValue::from_str(&mime).map_err(|_| attachments::refused())?;
    Ok(mime)
}

/// Detect identity/relationship drift without fetching ciphertext. Authenticated
/// decryption verifies content once; the immutable buffer's plaintext SHA-256 is
/// bound to confirmation. Re-encryption must not invalidate an approved buffer.
fn identity_projection() -> bson::Document {
    doc! {"_id":1,"user_id":1,"conversation_id":1,"group_id":1,"turn_id":1,"origin":1,"label":1,"size":1,"content_type":1,"chunks":1,"message_id":1,"parent_attachment_id":1,"chunk_index":1}
}
pub(crate) async fn metadata_identity(db: &Database, id: &str) -> AppResult<String> {
    let mut rows = db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .find(doc! {"$or":[{"_id":id},{"parent_attachment_id":id}]})
        .projection(identity_projection())
        .sort(doc! {"_id":1})
        .await?;
    let mut hash = Sha256::new();
    let mut count = 0;
    while let Some(row) = rows.try_next().await? {
        // Stable field order also tolerates whole-document re-encryption writes.
        let identity: std::collections::BTreeMap<_, _> = row.into_iter().collect();
        hash.update(bson::to_vec(&identity).map_err(|_| attachments::refused())?);
        count += 1;
    }
    if count == 0 {
        return Err(attachments::refused());
    }
    Ok(hex::encode(hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_attachment_identity_projection_never_reads_payload_or_encryption_versions() {
        let projection = identity_projection();
        for key in [
            "data_encrypted",
            "text_encrypted",
            "secret_context_versions",
        ] {
            assert!(!projection.contains_key(key));
        }
        for key in [
            "_id",
            "user_id",
            "conversation_id",
            "turn_id",
            "origin",
            "size",
            "content_type",
            "chunks",
            "parent_attachment_id",
            "chunk_index",
        ] {
            assert_eq!(projection.get_i32(key).unwrap(), 1);
        }
    }

    #[test]
    fn raw_attachment_provider_fields_in_composed_schemas_keep_legacy_arguments() {
        for schema in [
            json!({"properties":{"attachment_id":{"type":"string"}}}),
            json!({"allOf":[{"properties":{"attachment_id":{"type":"string"}}}]}),
            json!({"oneOf":[{"properties":{"attachment_id":{"type":"string"}}}]}),
            json!({"$ref":"#/$defs/body","$defs":{"body":{"properties":{"attachment_id":{"type":"string"}}}}}),
        ] {
            let endpoint = McpToolEndpoint {
                request_body_schema: Some(schema),
                ..Default::default()
            };
            assert!(!has_handle(
                &endpoint,
                &json!({"attachment_id":"provider-value"})
            ));
        }
        let endpoint = McpToolEndpoint::default();
        assert!(has_handle(
            &endpoint,
            &json!({"attachment_id":"file-handle"})
        ));
        assert!(has_handle(
            &endpoint,
            &json!({"attachment_id":"file-handle","body":"bytes"})
        ));
    }

    #[test]
    fn raw_attachment_schema_preserves_contract_and_declared_parameters() {
        let endpoint = McpToolEndpoint {
            endpoint_id: Uuid::new_v4().to_string(),
            method: "POST".into(),
            path: "/upload".into(),
            request_content_type: Some("application/octet-stream".into()),
            request_body_required: true,
            request_body_schema: Some(json!({"type":"string","format":"binary"})),
            parameters: Some(
                json!([{"name":"fileName","in":"query","required":true,"schema":{"type":"string"}}]),
            ),
            ..Default::default()
        };
        let before = mcp::endpoint_contract_digest(&endpoint);
        let normal = mcp::build_input_schema(&endpoint);
        let schema = input_schema(&endpoint);
        assert!(normal["properties"]["body"].is_object());
        let mut body = schema["properties"]["body"].clone();
        body["description"] = normal["properties"]["body"]["description"].clone();
        assert_eq!(body, normal["properties"]["body"]);
        assert_eq!(schema["required"], json!(["fileName"]));
        assert_eq!(
            schema["oneOf"],
            json!([{"required":["body"]},{"required":["attachment_id"]}])
        );
        assert_eq!(
            schema["properties"]["fileName"],
            normal["properties"]["fileName"]
        );
        assert_eq!(before, mcp::endpoint_contract_digest(&endpoint));
        assert_eq!(normal, mcp::build_input_schema(&endpoint));
        assert!(endpoint.attachment_input.is_none());
        assert_eq!(
            upload_content_type(&endpoint, "application/pdf").unwrap(),
            "application/octet-stream"
        );
        let mut wildcard = endpoint;
        wildcard.request_content_type = Some("*/*".into());
        assert_eq!(
            upload_content_type(&wildcard, "application/pdf").unwrap(),
            "application/pdf"
        );
    }
}
