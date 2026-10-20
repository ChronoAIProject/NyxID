//! Managed Base writes. Receipt values and attachment bytes never enter the ledger.
use super::{
    assistant_acknowledgement_service::ChatAuthority,
    assistant_upload_service as uploads, lark_attachment_tool as attachment,
    mcp_service::{self as mcp, McpToolEndpoint, McpToolService, McpToolSource},
};
use crate::{
    errors::{AppError, AppResult},
    models::lark_base_write::{LarkBaseWrite, UploadedFile, WritePrincipal, WriteState},
};
use chrono::Utc;
use mongodb::{Database, bson::doc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

mod ledger;
mod validation;
pub(crate) use ledger::{effect_gate, sweep};
const COLLECTION: &str = crate::models::lark_base_write::COLLECTION_NAME;
fn invalid() -> AppError {
    AppError::BadRequest(
        "Invalid managed Base write; fields and nonempty attachments are required".into(),
    )
}

pub(crate) struct InputUpload {
    id: String,
    sha256: String,
    size: usize,
}
pub(crate) struct Prepared {
    pub(crate) chat: ChatAuthority,
    app: String,
    table: String,
    fields: Value,
    assignments: BTreeMap<String, Vec<String>>,
    uploads: Vec<InputUpload>,
    pub(crate) contract_digests: BTreeMap<String, String>,
}

pub(crate) async fn contracts(
    db: &Database,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
) -> AppResult<BTreeMap<String, String>> {
    use crate::models::{
        downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService},
        service_endpoint::{COLLECTION_NAME as ENDPOINTS, PublicationState, ServiceEndpoint},
    };
    use futures::TryStreamExt;
    let McpToolSource::UserManaged {
        catalog_service_id: Some(catalog),
        user_service_id,
        ..
    } = &service.source
    else {
        return Err(attachment::refused());
    };
    let catalog = db
        .collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"_id":catalog,"is_active":true})
        .await?
        .ok_or_else(attachment::refused)?;
    let instance = super::user_service_service::find_user_service_by_id(db, user_service_id)
        .await?
        .filter(|s| s.is_active)
        .ok_or_else(attachment::refused)?;
    let mounted = super::service_history::collection::<crate::models::user_endpoint::UserEndpoint>(
        db,
        "user_endpoints",
    )
    .find_one(doc! {"_id":&instance.endpoint_id})
    .await?
    .ok_or_else(attachment::refused)?;
    if mounted.openapi_spec_url.is_some()
        || instance.catalog_service_id.as_deref() != Some(&catalog.id)
    {
        return Err(attachment::refused());
    }
    let rows: Vec<ServiceEndpoint> = db
        .collection::<ServiceEndpoint>(ENDPOINTS)
        .find(doc! {"service_id":&catalog.id,"is_active":true,"publication":"published"})
        .await?
        .try_collect()
        .await?;
    if rows
        .iter()
        .any(|row| row.publication != PublicationState::Published)
    {
        return Err(attachment::refused());
    }
    let mut loaded = mcp::service_endpoints_to_mcp(&rows.iter().collect::<Vec<_>>());
    attachment::stored_projections(Some(&catalog), &mut loaded);
    let current = loaded
        .iter()
        .find(|ep| ep.endpoint_id == endpoint.endpoint_id)
        .ok_or_else(attachment::refused)?;
    let Some(attachment::Projection::Managed(mark)) = &current.attachment_input else {
        return Err(attachment::refused());
    };
    if mcp::endpoint_contract_digest(current) != mcp::endpoint_contract_digest(endpoint) {
        return Err(attachment::refused());
    }
    let mut digests = mark.contract_digests.clone();
    digests.insert(
        attachment::COMPOSITE_OPERATION.into(),
        mcp::endpoint_contract_digest(current),
    );
    Ok(digests)
}

pub(crate) async fn prepare(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    chat: Option<&ChatAuthority>,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
    args: &Value,
    body: &Value,
) -> AppResult<Prepared> {
    if !matches!(
        endpoint.attachment_input,
        Some(attachment::Projection::Managed(_))
    ) || endpoint.async_operation.is_some()
        || !attachment::writes_enabled(db, chat).await?
    {
        return Err(attachment::refused());
    }
    let chat = chat
        .filter(|chat| !chat.guest)
        .ok_or_else(attachment::refused)?;
    super::assistant_resource_service::require_live(db, chat).await?;
    let app = args["app_token"].as_str().ok_or_else(invalid)?.to_owned();
    let table = args["table_id"].as_str().ok_or_else(invalid)?.to_owned();
    validation::token(&app)?;
    validation::token(&table)?;
    let (fields, assignments) = validation::body(body)?;
    let contract_digests = contracts(db, service, endpoint).await?;
    let ids: HashSet<_> = assignments.values().flatten().cloned().collect();
    let mut inputs = Vec::new();
    let mut total = 0usize;
    let mut sources = BTreeMap::new();
    let mut contents = HashSet::new();
    let mut ordered: Vec<_> = ids.iter().collect();
    ordered.sort();
    for id in ordered {
        let row = uploads::for_egress(db, chat, id).await?;
        let size = usize::try_from(row.size())
            .ok()
            .filter(|size| *size > 0 && *size <= super::attachment_extraction::MAX_BYTES)
            .ok_or_else(invalid)?;
        total = total
            .checked_add(size)
            .filter(|total| *total <= 50 * 1024 * 1024)
            .ok_or_else(invalid)?;
        let stored = match &row {
            uploads::EgressAttachment::User(row) => {
                if let Some(source) = &row.source {
                    sources.insert(source.event_key.clone(), source.declared);
                }
                row.content_sha256.clone()
            }
            uploads::EgressAttachment::Tool(_) => None,
        };
        let sha256 = if let Some(hash) = stored {
            hash
        } else {
            let _reservation = uploads::byte_budget::reserve(size).await?;
            let bytes = uploads::egress_bytes(db, keys, chat, &row).await?;
            hex::encode(Sha256::digest(&bytes))
        };
        if sha256.len() != 64
            || !sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !contents.insert(sha256.clone())
        {
            return Err(AppError::BadRequest(
                "Repeated or invalid photo content in Base write".into(),
            ));
        }
        inputs.push(InputUpload {
            id: id.clone(),
            sha256,
            size,
        });
    }
    use futures::TryStreamExt;
    for (event, declared) in sources {
        let roots: Vec<crate::models::assistant_upload::AssistantUpload> = db
            .collection(crate::models::assistant_upload::COLLECTION_NAME)
            .find(doc! {"user_id":&chat.user_id,"origin":"user_upload","source.event_key":&event})
            .limit(11)
            .await?
            .try_collect()
            .await?;
        if declared == 0
            || roots.len() != declared
            || roots.iter().any(|root| {
                !ids.contains(&root.id)
                    || root
                        .source
                        .as_ref()
                        .is_none_or(|source| source.declared != declared)
            })
        {
            return Err(AppError::BadRequest(format!(
                "{} of {} photos from that message could not be read; ask the owner to resend",
                declared.saturating_sub(roots.iter().filter(|root| ids.contains(&root.id)).count()),
                declared
            )));
        }
    }
    Ok(Prepared {
        chat: chat.clone(),
        app,
        table,
        fields,
        assignments,
        uploads: inputs,
        contract_digests,
    })
}

fn stable_key(owner: &str, app: &str, table: &str, hashes: &[String]) -> String {
    let mut hashes = hashes.to_vec();
    hashes.sort();
    hex::encode(Sha256::digest(format!(
        "v1\0{owner}\0{app}\0{table}\0{}",
        hashes.join("\0")
    )))
}

pub(crate) mod transport;
use super::assistant_service_execution::LarkManagedGuard;
use transport::{Reply, Transport};

enum RunError {
    NotSent(AppError),
    Other(AppError),
}

impl From<AppError> for RunError {
    fn from(error: AppError) -> Self {
        Self::Other(error)
    }
}

pub(crate) fn execute<'a>(
    db: &'a Database,
    keys: &'a crate::crypto::aes::EncryptionKeys,
    prepared: &'a Prepared,
    guard: &'a LarkManagedGuard<'a>,
    transport: Transport<'a>,
) -> futures::future::BoxFuture<'a, AppResult<Value>> {
    // Construct the managed future away from the shared MCP poll frame.
    Box::pin(execute_inner(db, keys, prepared, guard, transport))
}

async fn execute_inner(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    prepared: &Prepared,
    guard: &LarkManagedGuard<'_>,
    transport: Transport<'_>,
) -> AppResult<Value> {
    guard.revalidate().await?;
    let principal: WritePrincipal = guard.principal().await?;
    let hashes: Vec<_> = prepared
        .uploads
        .iter()
        .map(|upload| upload.sha256.clone())
        .collect();
    let photos: Vec<_> = hashes
        .iter()
        .map(|hash| {
            stable_key(
                &prepared.chat.user_id,
                &prepared.app,
                &prepared.table,
                std::slice::from_ref(hash),
            )
        })
        .collect();
    let now = Utc::now();
    let proposal = LarkBaseWrite {
        id: Uuid::new_v4().to_string(),
        key: stable_key(
            &prepared.chat.user_id,
            &prepared.app,
            &prepared.table,
            &hashes,
        ),
        user_id: prepared.chat.user_id.clone(),
        attempt: Uuid::new_v4().to_string(),
        state: WriteState::Uploading,
        principal,
        contract_digests: prepared.contract_digests.clone(),
        upload_ids: prepared
            .uploads
            .iter()
            .map(|upload| upload.id.clone())
            .collect(),
        files: Vec::new(),
        logical_request_sha256: validation::logical_digest(
            &prepared.fields,
            &prepared.assignments,
            &prepared.uploads,
        )?,
        create_body_sha256: None,
        client_token: Uuid::new_v4().to_string(),
        record_id: None,
        last_code: None,
        created_at: now,
        updated_at: now,
        deadline_at: now + chrono::Duration::seconds(180),
        expires_at: None,
    };
    let mut row = match ledger::claim(db, &proposal, &photos).await? {
        ledger::Claim::Replay(record) => {
            return Ok(json!({"record_id":record,"already_created":true}));
        }
        ledger::Claim::Active(row) if row.state == WriteState::Uploading => *row,
        ledger::Claim::Active(_) => {
            return Err(AppError::Conflict(
                "Base write outcome unknown; check the Base; NyxID will not create it again".into(),
            ));
        }
    };
    let work = tokio::time::timeout(
        (row.deadline_at - Utc::now()).to_std().unwrap_or_default(),
        run(db, keys, prepared, guard, &transport, &mut row, &photos),
    )
    .await;
    match work {
        Ok(Ok(value)) => Ok(value),
        outcome => {
            let (state, code) = if matches!(&outcome, Ok(Err(RunError::NotSent(_)))) {
                (WriteState::FailedDefinite, "not_sent")
            } else if row.state == WriteState::Uploading {
                (WriteState::FailedPreDispatch, "pre_dispatch")
            } else {
                (WriteState::Uncertain, "outcome_unknown")
            };
            // Cleanup lives outside the timed work. Process death/Stop is handled
            // by the flag-independent sweeper using these same attempt fences.
            ledger::terminal(db, &row, state, code, None).await?;
            match outcome {
                Ok(Err(RunError::NotSent(error) | RunError::Other(error))) => Err(error),
                _ => Err(AppError::Conflict(
                    "Base write timed out; check its recorded outcome before retrying".into(),
                )),
            }
        }
    }
}

async fn field_schema(
    db: &Database,
    prepared: &Prepared,
    guard: &LarkManagedGuard<'_>,
    transport: &Transport<'_>,
    row: &LarkBaseWrite,
) -> AppResult<BTreeMap<String, Value>> {
    let path = format!(
        "/bitable/v1/apps/{}/tables/{}/fields",
        prepared.app, prepared.table
    );
    let mut schema = BTreeMap::new();
    let mut next: Option<String> = None;
    let mut seen = HashSet::new();
    for _ in 0..5 {
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("page_size", "100");
            if let Some(token) = &next {
                query.append_pair("page_token", token);
            }
            query.finish()
        };
        let response = transport
            .send(
                guard,
                row,
                WriteState::Uploading,
                reqwest::Method::GET,
                &path,
                Some(&query),
                None,
                "application/json",
            )
            .await
            .map_err(|error| error.0)?;
        effect_gate(db, row, WriteState::Uploading).await?;
        let Reply::Json(status, body) = response else {
            return Err(AppError::BadRequest(
                "Base field schema could not be read".into(),
            ));
        };
        if !(200..300).contains(&status) || body["code"].as_i64() != Some(0) {
            return Err(permission_error(
                transport.target,
                body["code"].as_i64(),
                "field read",
            ));
        }
        let items = body["data"]["items"]
            .as_array()
            .filter(|items| items.len() <= 100)
            .ok_or_else(invalid)?;
        for item in items {
            let name = item["field_name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(invalid)?;
            if schema.insert(name.to_owned(), item.clone()).is_some() {
                return Err(invalid());
            }
        }
        if prepared
            .assignments
            .keys()
            .chain(prepared.fields.as_object().ok_or_else(invalid)?.keys())
            .all(|field| schema.contains_key(field))
        {
            validation::validate_fields(&prepared.fields, &prepared.assignments, &schema)?;
            return Ok(schema);
        }
        match body["data"]["has_more"].as_bool() {
            Some(false) => break,
            Some(true) => {
                let token = body["data"]["page_token"].as_str().ok_or_else(invalid)?;
                validation::token(token)?;
                if !seen.insert(token.to_owned()) {
                    return Err(invalid());
                }
                next = Some(token.to_owned());
            }
            None => return Err(invalid()),
        }
    }
    Err(AppError::BadRequest(
        "Base field schema is incomplete or a referenced field does not exist".into(),
    ))
}

fn permission_error(
    target: &super::proxy_service::ProxyTarget,
    code: Option<i64>,
    action: &str,
) -> AppError {
    // Provider prose can include account or request data. Only the numeric code
    // and the outbound connection label identify the permission principal.
    AppError::BadRequest(format!(
        "Lark {} failed (code {}), outbound service {} connection {}; verify its scopes and Base access",
        action,
        code.map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".into()),
        target.service.slug,
        target.service.name
    ))
}

async fn run(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    prepared: &Prepared,
    guard: &LarkManagedGuard<'_>,
    transport: &Transport<'_>,
    row: &mut LarkBaseWrite,
    photos: &[String],
) -> Result<Value, RunError> {
    field_schema(db, prepared, guard, transport, row).await?;
    for input in &prepared.uploads {
        if row
            .files
            .iter()
            .any(|file| file.content_sha256 == input.sha256)
        {
            continue;
        }
        let attachment = uploads::for_egress(db, &prepared.chat, &input.id).await?;
        let reservation = uploads::byte_budget::reserve(input.size).await?;
        effect_gate(db, row, WriteState::Uploading).await?;
        let bytes = uploads::egress_bytes(db, keys, &prepared.chat, &attachment).await?;
        if hex::encode(Sha256::digest(&bytes)) != input.sha256 {
            return Err(invalid().into());
        }
        let args = json!({"attachment_id":input.id,"parent_type":if attachment.content_type().starts_with("image/") {"bitable_image"}else{"bitable_file"},"parent_node":prepared.app});
        let fields = attachment::multipart::fields(&args, attachment.label(), input.size)?;
        let (body, content_type) =
            attachment::multipart::encode(&fields, attachment.content_type(), &bytes)?;
        drop(bytes);
        let _serial = credential_lock(&row.principal.credential_id).await;
        let mut token = None;
        for attempt in 0..=3 {
            uploads::for_egress(db, &prepared.chat, &input.id).await?;
            let reply = transport
                .send(
                    guard,
                    row,
                    WriteState::Uploading,
                    reqwest::Method::POST,
                    attachment::UPLOAD_PATH,
                    None,
                    Some(body.clone()),
                    &content_type,
                )
                .await
                .map_err(|error| error.0)?;
            effect_gate(db, row, WriteState::Uploading).await?;
            let retry = match reply {
                Reply::Json(status, reply) => {
                    let code = reply["code"].as_i64();
                    if (200..300).contains(&status) && code == Some(0) {
                        token = reply["data"]["file_token"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        if token.is_none() {
                            return Err(AppError::BadRequest(
                                "Base upload returned no usable file_token".into(),
                            )
                            .into());
                        }
                        false
                    } else if matches!(code, Some(1061045 | 1061006)) {
                        true
                    } else {
                        return Err(permission_error(transport.target, code, "upload").into());
                    }
                }
                Reply::Transport => true,
            };
            if token.is_some() {
                break;
            }
            if !retry || attempt == 3 {
                return Err(AppError::BadRequest(
                    "Base upload rate-limited or unavailable; no record was created".into(),
                )
                .into());
            }
            let jitter = rand::random::<u8>() as u64;
            tokio::time::sleep(std::time::Duration::from_millis(
                50 * (1 << attempt) + jitter,
            ))
            .await;
            effect_gate(db, row, WriteState::Uploading).await?;
        }
        let file = UploadedFile {
            upload_id: input.id.clone(),
            content_sha256: input.sha256.clone(),
            file_token: token.ok_or_else(invalid)?,
        };
        ledger::save_file(db, row, &file).await?;
        row.files.push(file);
        drop(reservation);
    }
    let mut fields = prepared.fields.as_object().ok_or_else(invalid)?.clone();
    for (field, ids) in &prepared.assignments {
        let values: AppResult<Vec<_>> = ids
            .iter()
            .map(|id| {
                let input = prepared
                    .uploads
                    .iter()
                    .find(|input| input.id == *id)
                    .ok_or_else(invalid)?;
                let file = row
                    .files
                    .iter()
                    .find(|file| file.content_sha256 == input.sha256)
                    .ok_or_else(invalid)?;
                Ok(json!({"file_token":file.file_token}))
            })
            .collect();
        fields.insert(field.clone(), json!(values?));
    }
    let body = mcp::canonical_json(json!({"fields":fields}));
    let encoded = serde_json::to_vec(&body).map_err(|_| invalid())?;
    guard.revalidate_effect(row, WriteState::Uploading).await?;
    ledger::barrier(db, row, photos, &hex::encode(Sha256::digest(&encoded))).await?;
    row.state = WriteState::Dispatching;
    let path = format!(
        "/bitable/v1/apps/{}/tables/{}/records",
        prepared.app, prepared.table
    );
    // [L1] client_token is a query parameter on Create record, not a field
    // in the record body. A fresh authorized attempt gets a fresh UUID.
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_token", &row.client_token)
        .finish();
    let reply = transport
        .send(
            guard,
            row,
            WriteState::Dispatching,
            reqwest::Method::POST,
            &path,
            Some(&query),
            Some(encoded.into()),
            "application/json",
        )
        .await
        .map_err(|error| RunError::NotSent(error.0))?;
    let (state, code, record) = classify(reply);
    if !ledger::terminal(db, row, state, &code, record.as_deref()).await? {
        return Err(AppError::Conflict(
            "Base write result was already settled; check the Base".into(),
        )
        .into());
    }
    if state == WriteState::Succeeded {
        return Ok(json!({"record_id":record,"already_created":false}));
    }
    Err((if state == WriteState::FailedDefinite {
        permission_error(transport.target, code.parse().ok(), "record create")
    } else {
        AppError::Conflict(
            "Base write outcome unknown; check the Base; NyxID will not create it again".into(),
        )
    })
    .into())
}

fn classify(reply: Reply) -> (WriteState, String, Option<String>) {
    let Reply::Json(status, body) = reply else {
        return (WriteState::Uncertain, "transport".into(), None);
    };
    let code = body["code"].as_i64();
    if (200..300).contains(&status)
        && code == Some(0)
        && let Some(record) = body["data"]["record"]["record_id"]
            .as_str()
            .filter(|s| !s.is_empty())
    {
        return (WriteState::Succeeded, "0".into(), Some(record.to_owned()));
    }
    // [L1] Create record: https://open.feishu.cn/document/server-docs/docs/bitable-v1/app-table-record/create
    // 1254001: invalid request; 1254004: invalid app token; 1254045: field missing;
    // 1254069: attachment field conversion; 1254027: permission denied;
    // 1254303: insufficient record permission; 1254302: forbidden operation.
    // Each documents refusal/non-application. All unlisted outcomes, including
    // 1254607/1254291/1255006/1254608 and every 5xx, remain uncertain [L1].
    let definite = status < 500
        && matches!(
            code,
            Some(1254001 | 1254004 | 1254045 | 1254069 | 1254027 | 1254303 | 1254302)
        );
    (
        if definite {
            WriteState::FailedDefinite
        } else {
            WriteState::Uncertain
        },
        code.map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".into()),
        None,
    )
}

async fn credential_lock(id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    use std::sync::{Arc, LazyLock, Mutex, Weak};
    type CredentialLocks = Mutex<std::collections::HashMap<String, Weak<tokio::sync::Mutex<()>>>>;
    static LOCKS: LazyLock<CredentialLocks> = LazyLock::new(|| Mutex::new(Default::default()));
    let lock = {
        let mut locks = LOCKS.lock().expect("credential locks");
        locks.retain(|_, weak| weak.strong_count() > 0);
        let lock = locks
            .get(id)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        locks.insert(id.into(), Arc::downgrade(&lock));
        lock
    };
    lock.lock_owned().await
}

#[cfg(test)]
mod tests;
