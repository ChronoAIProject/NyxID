//! Live-chat image publication. The adapter owns the wire format; this host owns
//! files, consent, exact connection authority and a durable no-resend barrier.
use super::{
    assistant_acknowledgement_service::ChatAuthority,
    assistant_upload_service as uploads,
    mcp_service::{self as mcp, McpToolEndpoint, McpToolService, McpToolSource},
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::DownstreamService,
        image_publication::{COLLECTION_NAME, ImagePublication},
        service_endpoint::{PublicationState, ServiceEndpoint},
    },
};
use chrono::Utc;
use mongodb::{
    Database,
    bson::{self, doc},
};
use nyxid_service_adapters::image_publication::{ImagePublisher, LinkedIn};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

mod ledger;
pub(crate) mod transport;
use ledger::{load, preview_claim, preview_key, update};
use transport::Transport;

pub const OPERATION: &str = "publish_image_attachment";
pub const PATH: &str = "/nyxid/connector/image-publication";
pub fn candidate(endpoint: &McpToolEndpoint) -> bool {
    matches!(
        endpoint.attachment_input,
        Some(super::lark_attachment_tool::Projection::ImagePublication)
    )
}
pub fn refused(reason: &str) -> AppError {
    AppError::Forbidden(format!("image_publication_{reason}"))
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) struct Connector {
    pub adapter: &'static dyn ImagePublisher,
    pub managed_spec_key: &'static str,
}
static LINKEDIN: Connector = Connector {
    adapter: &LinkedIn,
    managed_spec_key: "linkedin",
};
#[cfg(test)]
static FIXTURE: Connector = Connector {
    adapter: &nyxid_service_adapters::image_publication::FixturePublisher,
    // The fixture deliberately shares the host's managed chat input contract;
    // all provider requests, recipients and receipts use its independent adapter.
    managed_spec_key: "linkedin",
};
pub(crate) fn connector(catalog: &DownstreamService) -> AppResult<&'static Connector> {
    if catalog.created_by != "system" || !catalog.is_active {
        return Err(refused("trusted_catalog_required"));
    }
    match catalog.slug.as_str() {
        "api-linkedin" => Ok(&LINKEDIN),
        #[cfg(test)]
        "fixture-publisher" => Ok(&FIXTURE),
        _ => Err(refused("unsupported_connector")),
    }
}

pub(crate) fn mark_catalog_operations(
    catalog: Option<&DownstreamService>,
    endpoints: &mut [McpToolEndpoint],
) {
    if catalog.is_some_and(|c| connector(c).is_ok()) {
        for endpoint in endpoints {
            if endpoint.name == OPERATION && endpoint.path == PATH {
                endpoint.attachment_input =
                    Some(super::lark_attachment_tool::Projection::ImagePublication);
            }
        }
    }
}

/// Merge only the compiled, published operation. Mounted text/analytics remain
/// authoritative for their own operations; they cannot author this capability.
pub(crate) fn append_catalog_operation(
    published: &mut Vec<McpToolEndpoint>,
    rows: &[&ServiceEndpoint],
    catalog: Option<&DownstreamService>,
) {
    if catalog.is_none_or(|c| connector(c).is_err()) {
        return;
    }
    if published
        .iter()
        .any(|ep| ep.name == OPERATION && !candidate(ep))
    {
        return;
    }
    published.retain(|ep| !candidate(ep));
    let mut managed = mcp::service_endpoints_to_mcp(
        &rows
            .iter()
            .copied()
            .filter(|row| row.name == OPERATION)
            .collect::<Vec<_>>(),
    );
    for endpoint in &mut managed {
        endpoint.attachment_input = Some(super::lark_attachment_tool::Projection::ImagePublication);
    }
    published.extend(managed);
}

async fn eligible_chat<'a>(
    db: &Database,
    chat: Option<&'a ChatAuthority>,
) -> AppResult<&'a ChatAuthority> {
    let chat = chat
        .filter(|c| !c.guest && !super::assistant_service_execution::is_shared_chat(c))
        .ok_or_else(|| refused("live_owner_chat_required"))?;
    super::assistant_resource_service::require_live(db, chat).await?;
    if !uploads::eligible_egress_key(db, chat).await? {
        return Err(refused("owner_key_required"));
    }
    Ok(chat)
}

pub(crate) async fn require_operation(
    db: &Database,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
) -> AppResult<&'static dyn ImagePublisher> {
    let McpToolSource::UserManaged {
        catalog_service_id: Some(catalog_id),
        user_service_id,
        has_server_credential: true,
        node_id,
        ..
    } = &service.source
    else {
        return Err(refused("catalog_connection_required"));
    };
    if node_id.as_deref().is_some_and(|s| !s.is_empty()) || endpoint.async_operation.is_some() {
        return Err(refused("hosted_synchronous_required"));
    }
    let catalog = db
        .collection::<DownstreamService>("downstream_services")
        .find_one(doc! {"_id":catalog_id,"is_active":true})
        .await?
        .ok_or_else(|| refused("catalog_unavailable"))?;
    let connector = connector(&catalog)?;
    let adapter = connector.adapter;
    let instance = super::user_service_service::find_user_service_by_id(db, user_service_id)
        .await?
        .filter(|s| s.is_active)
        .ok_or_else(|| refused("connection_unavailable"))?;
    if instance.catalog_service_id.as_deref() != Some(catalog_id)
        || super::platform_key_service::binding(&instance) != "user"
        || instance.auth_method != "bearer"
        || instance.node_id.as_deref().is_some_and(|s| !s.is_empty())
    {
        return Err(refused("byo_bearer_connection_required"));
    }
    let credential = instance
        .api_key_id
        .as_deref()
        .ok_or_else(|| refused("byo_credential_required"))?;
    if db.collection::<bson::Document>("user_api_keys").find_one(doc! {"_id":credential,"user_id":&instance.user_id,"status":"active","credential_type":{"$nin":["node_managed","ssh_certificate"]}}).await?.is_none() {return Err(refused("byo_credential_required"))}
    let mounted = super::service_history::collection::<crate::models::user_endpoint::UserEndpoint>(
        db,
        "user_endpoints",
    )
    .find_one(doc! {"_id":&instance.endpoint_id})
    .await?
    .ok_or_else(|| refused("endpoint_unavailable"))?;
    validate_origin(adapter, &catalog.base_url)?;
    validate_origin(adapter, &mounted.url)?;
    let row = db
        .collection::<ServiceEndpoint>("service_endpoints")
        .find_one(doc! {"_id":&endpoint.endpoint_id,"service_id":catalog_id,"is_active":true})
        .await?
        .ok_or_else(|| refused("operation_unavailable"))?;
    if row.name != OPERATION
        || row.publication != PublicationState::Published
        || row.operation_generation <= 0
        || row.execution != crate::models::service_endpoint::ExecutionKind::HttpOperation
    {
        return Err(refused("operation_not_published"));
    }
    if service
        .durable_endpoint_metadata
        .get(&row.id)
        .is_none_or(|meta| meta.operation_generation != row.operation_generation)
    {
        return Err(refused("operation_generation_changed"));
    }
    let mut loaded = mcp::service_endpoints_to_mcp(&[&row])
        .pop()
        .ok_or_else(|| refused("contract"))?;
    let spec = super::catalog_spec_registry::spec_for_key(connector.managed_spec_key)
        .ok_or_else(|| refused("compiled_contract"))?;
    let input = super::catalog_spec_sync::destination_endpoint_inputs(&catalog, &spec)?
        .into_iter()
        .find(|i| i.name == OPERATION)
        .ok_or_else(|| refused("compiled_contract"))?;
    let mut expected = super::service_endpoint_service::endpoint_from_input(
        &catalog.id,
        input,
        PublicationState::Published,
    );
    expected.id.clone_from(&row.id);
    let mut expected = mcp::service_endpoints_to_mcp(&[&expected])
        .pop()
        .ok_or_else(|| refused("compiled_contract"))?;
    loaded.attachment_input = Some(super::lark_attachment_tool::Projection::ImagePublication);
    expected.attachment_input = Some(super::lark_attachment_tool::Projection::ImagePublication);
    if !candidate(endpoint)
        || mcp::endpoint_contract_digest(&loaded) != mcp::endpoint_contract_digest(endpoint)
        || mcp::endpoint_contract_digest(&loaded) != mcp::endpoint_contract_digest(&expected)
    {
        return Err(refused("contract_changed"));
    }
    Ok(adapter)
}

fn validate_origin(adapter: &dyn ImagePublisher, raw: &str) -> AppResult<()> {
    if raw.trim_end_matches('/') == adapter.api_origin() {
        return Ok(());
    }
    #[cfg(test)]
    if super::resource_adapter::test_media_origin(raw).is_some() {
        return Ok(());
    }
    Err(refused("api_destination"))
}

pub async fn filter_discovery(
    db: &Database,
    chat: Option<&ChatAuthority>,
    services: &mut [McpToolService],
) -> AppResult<()> {
    if !services.iter().any(|s| s.endpoints.iter().any(candidate)) {
        return Ok(());
    }
    let eligible = eligible_chat(db, chat).await.is_ok();
    for service in services {
        let mut keep = Vec::new();
        for endpoint in &service.endpoints {
            if !candidate(endpoint)
                || eligible && require_operation(db, service, endpoint).await.is_ok()
            {
                keep.push(endpoint.clone());
            }
        }
        service.endpoints = keep;
    }
    Ok(())
}

pub(crate) struct Prepared {
    pub chat: ChatAuthority,
    pub action: String,
    pub company: String,
    pub company_url: Option<String>,
    pub caption: String,
    pub attachment: String,
    pub sha256: String,
    pub identity: String,
    pub authority: String,
    pub contract: String,
    pub draft: Option<ImagePublication>,
    pub bytes: bytes::Bytes,
    pub mime: String,
    pub name: String,
    pub input: Value,
    pub request_id: String,
    pub transfer: bool,
    _reservation: Option<uploads::byte_budget::ByteReservation>,
}
impl Prepared {
    pub fn publish(&self) -> bool {
        self.action == "publish" && self.draft.as_ref().is_some_and(|d| d.state == "ready")
    }
    pub fn upload(&self) -> bool {
        self.action == "preview" && self.transfer
    }
    pub fn read_only(&self) -> bool {
        match self.action.as_str() {
            "status" | "cancel" => true,
            "preview" => !self.upload(),
            "publish" => !self.publish(),
            _ => false,
        }
    }
    pub fn review(
        &self,
    ) -> Option<crate::models::assistant_acknowledgement::ImagePublicationReview> {
        if !self.publish() {
            return None;
        }
        let draft = self.draft.as_ref()?;
        Some(
            crate::models::assistant_acknowledgement::ImagePublicationReview {
                draft_id: draft.id.clone(),
                company: self.company.clone(),
                company_url: self.company_url.clone(),
                caption: self.caption.clone(),
                attachment_id: self.attachment.clone(),
                conversation_id: self.chat.conversation_id.clone(),
                content_type: self.mime.clone(),
                label: self.name.clone(),
            },
        )
    }
    pub fn binding(&self) -> Value {
        json!({
            "action":self.action,
            "company":self.company,
            "caption":self.caption,
            "attachment_id":self.attachment,
            "content_sha256":self.sha256,
            "metadata_identity":self.identity,
            "authority":self.authority,
            "contract":self.contract,
            "draft_id":self.draft.as_ref().map(|draft| &draft.id),
            "prepared_asset_digest": self.draft.as_ref().and_then(|draft|draft.image.as_ref()).map(|image|hash(image.as_bytes())),
        })
    }
    pub fn image_url(&self) -> String {
        format!(
            "/api/v1/assistant/nyxagent/conversations/{}/attachments/{}",
            self.chat.conversation_id, self.attachment
        )
    }
    pub fn summary(&self) -> String {
        format!(
            "{} image attachment\nCompany page: {}\nFull caption:\n{}\nImage: {} ({} bytes)\n{}",
            if self.publish() { "Publish" } else { "Prepare" },
            self.company,
            self.caption,
            self.name,
            self.bytes.len(),
            self.image_url()
        )
    }
}

pub(crate) async fn prepare(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    chat: Option<&ChatAuthority>,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
    body: &Value,
) -> AppResult<Prepared> {
    let chat = eligible_chat(db, chat).await?;
    let adapter = require_operation(db, service, endpoint).await?;
    let obj = body.as_object().ok_or_else(|| refused("arguments"))?;
    if obj.keys().any(|k| {
        ![
            "action",
            "company",
            "caption",
            "attachment_id",
            "draft_id",
            "request_id",
        ]
        .contains(&k.as_str())
    }) {
        return Err(refused("unknown_argument"));
    }
    let action = body["action"]
        .as_str()
        .filter(|a| ["preview", "publish", "status", "cancel"].contains(a))
        .ok_or_else(|| refused("action"))?;
    let request_id = body["request_id"].as_str().unwrap_or_default().to_owned();
    if action == "preview" && Uuid::parse_str(&request_id).is_err() {
        return Err(refused("preview_request_id_required"));
    }
    let draft = if let Some(id) = body["draft_id"].as_str() {
        Some(db.collection::<ImagePublication>(COLLECTION_NAME).find_one(doc! {"_id":id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"connection_id":&service.service_id}).await?.ok_or_else(||refused("draft_not_in_this_chat"))?)
    } else {
        None
    };
    if action != "preview" && draft.is_none() {
        return Err(refused("draft_required"));
    }
    let company = body["company"]
        .as_str()
        .or_else(|| draft.as_ref().map(|d| d.company.as_str()))
        .filter(|s| adapter.company(s))
        .ok_or_else(|| refused("company"))?
        .to_owned();
    let caption = body["caption"].as_str().unwrap_or_default().to_owned();
    if matches!(action, "publish" | "preview")
        && (caption.trim().is_empty()
            || caption.chars().count() > 3000
            || caption
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'))
    {
        return Err(refused("caption"));
    }
    let attachment = body["attachment_id"]
        .as_str()
        .or_else(|| draft.as_ref().map(|d| d.attachment_id.as_str()))
        .unwrap_or_default()
        .to_owned();
    let authority = super::assistant_service_execution::connection_binding(
        db,
        &service.service_id,
        &chat.api_key_id,
    )
    .await?;
    let mut prepared = Prepared {
        chat: chat.clone(),
        action: action.into(),
        company_url: adapter.company_url(&company),
        company,
        caption,
        attachment,
        sha256: String::new(),
        identity: String::new(),
        authority,
        contract: mcp::canonical_sha256(
            json!({"endpoint":mcp::endpoint_contract_digest(endpoint),"generation":service.durable_endpoint_metadata.get(&endpoint.endpoint_id).map(|m|m.operation_generation)}),
        ),
        draft,
        bytes: bytes::Bytes::new(),
        mime: String::new(),
        name: String::new(),
        _reservation: None,
        input: body.clone(),
        request_id,
        transfer: action == "preview",
    };
    if action == "preview"
        && let Some(existing) = db
            .collection::<ImagePublication>(COLLECTION_NAME)
            .find_one(doc! {"key":preview_key(chat,&service.service_id,&prepared.request_id)})
            .await?
    {
        if existing.company != prepared.company
            || existing.caption_sha256 != hash(prepared.caption.as_bytes())
            || existing.attachment_id != prepared.attachment
            || existing.authority != prepared.authority
            || existing.contract != prepared.contract
        {
            return Err(refused("request_id_payload_changed"));
        }
        // Retrying an already-uploaded/preparing revision is a state read.
        // A consumed file-egress card cannot block recovery of its receipt.
        if existing.state != "upload_failed"
            && !(existing.state == "preparing" && existing.lease_until <= Utc::now())
        {
            prepared.transfer = false;
            prepared.sha256 = existing.content_sha256;
            prepared.identity = existing.metadata_identity;
        }
    }
    if prepared.upload()
        || action == "publish" && prepared.draft.as_ref().is_some_and(|d| d.state == "ready")
    {
        let identity =
            super::raw_attachment_tool::metadata_identity(db, &prepared.attachment).await?;
        let file = uploads::for_egress(db, chat, &prepared.attachment).await?;
        if !matches!(file.content_type(), "image/png" | "image/jpeg") {
            return Err(refused("png_or_jpeg_required"));
        }
        let size = usize::try_from(file.size())
            .ok()
            .filter(|n| *n > 0 && *n <= super::attachment_extraction::MAX_BYTES)
            .ok_or_else(|| refused("file_size"))?;
        let reservation = uploads::byte_budget::reserve(size).await?;
        let bytes = uploads::egress_bytes(db, keys, chat, &file).await?;
        let format = image::guess_format(&bytes).map_err(|_| refused("image_content"))?;
        if !matches!(
            (file.content_type(), format),
            ("image/png", image::ImageFormat::Png) | ("image/jpeg", image::ImageFormat::Jpeg)
        ) {
            return Err(refused("image_content"));
        }
        if identity
            != super::raw_attachment_tool::metadata_identity(db, &prepared.attachment).await?
        {
            return Err(refused("file_changed"));
        }
        prepared.sha256 = hash(&bytes);
        prepared.identity = identity;
        prepared.mime = file.content_type().into();
        prepared.name = file.label().into();
        prepared.bytes = bytes.into();
        prepared._reservation = Some(reservation);
    }
    if action == "preview"
        && prepared.draft.as_ref().is_some_and(|old| {
            matches!(
                old.state.as_str(),
                "ready" | "cancelled" | "superseded" | "processing" | "failed"
            ) && old.image.is_some()
                && (old.uploaded || old.state == "ready")
                && old.company == prepared.company
                && old.authority == prepared.authority
                && old.contract == prepared.contract
                && old.content_sha256 == prepared.sha256
        })
    {
        prepared.transfer = false;
    }
    if action == "publish" {
        let draft = prepared
            .draft
            .as_ref()
            .ok_or_else(|| refused("draft_required"))?;
        if draft.state == "ready"
            && (draft.company != prepared.company
                || draft.caption_sha256 != hash(prepared.caption.as_bytes())
                || draft.content_sha256 != prepared.sha256
                || draft.metadata_identity != prepared.identity
                || draft.attachment_id != prepared.attachment
                || draft.authority != prepared.authority
                || draft.contract != prepared.contract)
        {
            return Err(refused("draft_changed_preview_again"));
        }
        if !matches!(
            draft.state.as_str(),
            "ready" | "created" | "unknown" | "failed" | "dispatching"
        ) || draft.image.is_none()
        {
            return Err(refused("draft_not_ready_or_cancelled"));
        }
    }
    Ok(prepared)
}

pub(crate) struct Guard<'a> {
    pub db: &'a Database,
    pub keys: &'a crate::crypto::aes::EncryptionKeys,
    pub admission: &'a super::assistant_service_execution::Admission,
    pub target: &'a super::proxy_service::ProxyTarget,
    pub service: &'a McpToolService,
    pub endpoint: &'a McpToolEndpoint,
    pub prepared: &'a Prepared,
    pub row: Option<(&'a str, &'a str, &'a str)>,
}
impl Guard<'_> {
    pub(crate) fn revalidate(&self) -> futures::future::BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            eligible_chat(self.db, Some(&self.prepared.chat)).await?;
            let adapter = require_operation(self.db, self.service, self.endpoint).await?;
            validate_origin(adapter, &self.target.base_url)?;
            self.admission
                .revalidate(self.db, &self.service.service_id, Some(self.target))
                .await?;
            super::assistant_service_execution::verify_resolved(
                self.db,
                self.keys,
                &self.prepared.chat,
                &self.service.service_id,
                self.target,
                "POST",
                PATH,
            )
            .await?;
            if super::assistant_service_execution::connection_binding(
                self.db,
                &self.service.service_id,
                &self.prepared.chat.api_key_id,
            )
            .await?
                != self.prepared.authority
            {
                return Err(refused("authority_changed_preview_again"));
            }
            let key = self
                .db
                .collection::<crate::models::api_key::ApiKey>("api_keys")
                .find_one(doc! {"_id":&self.prepared.chat.api_key_id,"is_active":true})
                .await?
                .ok_or_else(|| refused("key_revoked"))?;
            let path = super::proxy_authorization::CanonicalPath::from_mcp_built(PATH)?;
            let catalog = super::agent_operation_scope_service::mcp_catalog_id(self.service);
            super::agent_operation_scope_service::authorize(
                &key.assistant_operation_scopes,
                &self.service.service_id,
                catalog,
                Some(&self.endpoint.endpoint_id),
                "POST",
                &path,
                false,
                false,
            )?;
            let body =
                serde_json::to_vec(&self.prepared.input).map_err(|_| refused("arguments"))?;
            super::agent_operation_scope_service::check_inputs(
                &key.assistant_operation_scopes,
                &self.service.service_id,
                catalog,
                Some(&self.endpoint.endpoint_id),
                "POST",
                &path,
                None,
                &body,
                Some("application/json"),
            )?;
            if self.prepared.upload() || self.prepared.publish() {
                uploads::for_egress(self.db, &self.prepared.chat, &self.prepared.attachment)
                    .await?;
                if super::raw_attachment_tool::metadata_identity(self.db, &self.prepared.attachment)
                    .await?
                    != self.prepared.identity
                {
                    return Err(refused("file_changed"));
                }
            }
            if let Some((id, state, attempt)) = self.row {
                ledger::effect_gate(self.db, &self.prepared.chat.user_id, id, state, attempt)
                    .await?;
            }
            super::assistant_resource_service::require_live(self.db, &self.prepared.chat).await
        })
    }
    fn for_row<'a>(&'a self, row: &'a ImagePublication, state: &'a str) -> Guard<'a> {
        Guard {
            db: self.db,
            keys: self.keys,
            admission: self.admission,
            target: self.target,
            service: self.service,
            endpoint: self.endpoint,
            prepared: self.prepared,
            row: Some((&row.id, state, &row.attempt)),
        }
    }
}

fn receipt(adapter: &dyn ImagePublisher, row: &ImagePublication) -> Value {
    let post_url = if row.state == "created" {
        row.post.as_deref().and_then(|post| adapter.feed_url(post))
    } else {
        None
    };
    // Preparation is a private provider capability. The agent receives only
    // the opaque draft and authenticated original until creation has a receipt.
    let published_image = if row.state == "created" && row.post.is_some() {
        row.image.as_deref()
    } else {
        None
    };
    let instructions = if matches!(row.state.as_str(), "unknown" | "dispatching") {
        "Outcome unknown; do not submit again. Reconcile the saved receipt or check the selected company page."
    } else if row.state == "created" && row.post.is_none() {
        "The provider accepted creation without a usable post ID. Do not create another post. Check the selected company page."
    } else {
        "Use status to check current visibility. A creation receipt is separate from current visibility."
    };
    json!({
        "draft_id": row.id,
        "state": row.state,
        "stage": row.stage,
        "http_status": row.http_status,
        "code": row.last_code,
        "post_urn": row.post,
        "post_url": post_url,
        "image": published_image,
        "company": row.company,
        "verification": row.verification,
        "posted_at": row.posted_at,
        "checked_at": row.checked_at,
        "last_verified_at": row.verified_at,
        "instructions": instructions,
    })
}

pub(crate) fn publication_gate(
    operation: &Value,
    gate: super::assistant_acknowledgement_service::Gate,
    facts: super::assistant_acknowledgement_service::ActionFacts,
    instruction_required: bool,
) -> super::assistant_acknowledgement_service::Gate {
    use super::assistant_acknowledgement_service::{
        ConfirmationNeed, Gate, OrgInstructionRequirement,
    };
    if !operation["image_publication_review"].is_object() {
        return gate;
    }
    let org_instruction = if instruction_required {
        OrgInstructionRequirement::Required
    } else {
        OrgInstructionRequirement::None
    };
    Gate::Ask(ConfirmationNeed {
        org_instruction,
        class: facts.class,
        mode: None,
        webhook: false,
        policy: true,
        delegation: false,
    })
}

pub(crate) struct ResultStatus {
    pub tool: u16,
    pub audit: u16,
}
pub(crate) fn result_status(
    action: &str,
    value: &Value,
    provider_effect: bool,
    provider_status: Option<u16>,
) -> ResultStatus {
    let recorded = value["http_status"]
        .as_u64()
        .and_then(|status| u16::try_from(status).ok());
    let refusal = recorded.filter(|status| (400..500).contains(status));
    let state = value["state"].as_str().unwrap_or_default();
    let status = match (action, state) {
        ("status" | "cancel", _) => 200,
        ("preview", "preparing" | "processing") => 202,
        ("preview", "ready") => 200,
        ("publish", "created") => {
            if provider_effect {
                recorded.filter(|s| (200..300).contains(s)).unwrap_or(200)
            } else {
                200
            }
        }
        (_, "upload_failed" | "failed") => refusal.unwrap_or(502),
        (_, "unknown" | "dispatching") => 409,
        _ => 409,
    };
    let audit = if action == "status" && provider_effect {
        provider_status.unwrap_or(502)
    } else if matches!(state, "unknown" | "dispatching") && provider_effect {
        recorded.or(provider_status).unwrap_or(504)
    } else {
        status
    };
    ResultStatus {
        tool: status,
        audit,
    }
}
fn definite_refusal(status: u16) -> bool {
    // Authentication, validation, unsupported method/media and throttling do
    // not admit a create. Timeouts, conflicts and every 5xx stay ambiguous.
    matches!(
        status,
        400 | 401 | 403 | 404 | 405 | 406 | 410 | 413 | 415 | 422 | 429
    )
}

pub(crate) async fn execute(guard: &Guard<'_>, transport: Transport<'_>) -> AppResult<Value> {
    guard.revalidate().await?;
    let adapter = require_operation(guard.db, guard.service, guard.endpoint).await?;
    match guard.prepared.action.as_str() {
        "preview" => {
            let (mut row, claimed) = preview_claim(guard, adapter).await?;
            if !claimed {
                let mut value = receipt(adapter, &row);
                if row.state == "ready" {
                    add_preview(&mut value, guard).await?;
                }
                return Ok(value);
            }
            let work = tokio::time::timeout(
                std::time::Duration::from_secs(75),
                prepare_image(guard, &transport, adapter, &mut row),
            )
            .await;
            match work {
                Ok(Ok(())) => {
                    update(guard.db, &mut row, "ready", "ready", Some(200)).await?;
                    let mut value = receipt(adapter, &row);
                    add_preview(&mut value, guard).await?;
                    Ok(value)
                }
                outcome => {
                    let code = match outcome {
                        Ok(Err(ref e)) => failure_code(e),
                        _ => "prepare_timeout",
                    };
                    let pending = row.state == "processing"
                        && matches!(
                            code,
                            "image_readiness_timeout" | "prepare_timeout" | "provider_transport"
                        );
                    row.lease_until = Utc::now();
                    row.last_code = Some(code.into());
                    let state = if pending {
                        "processing"
                    } else {
                        "upload_failed"
                    };
                    let stage = row.stage.clone();
                    let status = row.http_status;
                    update(guard.db, &mut row, state, &stage, status).await?;
                    Ok(receipt(adapter, &row))
                }
            }
        }
        "cancel" => {
            let mut row = load(
                guard.db,
                &guard
                    .prepared
                    .draft
                    .as_ref()
                    .ok_or_else(|| refused("draft_required"))?
                    .id,
            )
            .await?;
            if matches!(
                row.state.as_str(),
                "ready" | "preparing" | "processing" | "upload_failed"
            ) {
                update(guard.db, &mut row, "cancelled", "cancelled", None).await?;
                ledger::retire_cards(guard.db, &row, None).await?;
            }
            Ok(receipt(adapter, &row))
        }
        "status" => {
            let mut row = load(
                guard.db,
                &guard
                    .prepared
                    .draft
                    .as_ref()
                    .ok_or_else(|| refused("draft_required"))?
                    .id,
            )
            .await?;
            if row.post.is_some()
                && matches!(row.state.as_str(), "created" | "unknown" | "dispatching")
            {
                reconcile(guard, &transport, adapter, &mut row).await?;
            } else if row.state == "processing" {
                let result = poll_ready(guard, &transport, adapter, &mut row).await;
                match result {
                    Ok(()) => update(guard.db, &mut row, "ready", "ready", Some(200)).await?,
                    Err(error) => {
                        let code = failure_code(&error);
                        row.last_code = Some(code.into());
                        let state = if code == "image_readiness_timeout" {
                            "processing"
                        } else {
                            "upload_failed"
                        };
                        let status = row.http_status;
                        update(guard.db, &mut row, state, "readiness", status).await?;
                    }
                }
            }
            Ok(receipt(adapter, &row))
        }
        "publish" => publish(guard, &transport, adapter).await,
        _ => Err(refused("action")),
    }
}
fn failure_code(error: &AppError) -> &'static str {
    if let AppError::Forbidden(message) = error {
        match message.strip_prefix("image_publication_") {
            Some("image_processing_failed") => "image_processing_failed",
            Some("image_readiness_timeout") => "image_readiness_timeout",
            Some("image_owner_mismatch" | "fixture_owner") => "image_owner_mismatch",
            Some("invalid_upload_recipient" | "upload_recipient_or_expiry") => {
                "upload_recipient_refused"
            }
            Some("upload_expired" | "missing_upload_expiry") => "upload_expired",
            Some("provider_transport_outcome_unknown" | "upload_transport") => "provider_transport",
            Some("provider_unreachable" | "upload_not_sent") => "not_sent",
            Some(
                "draft_cancelled_or_changed"
                | "authority_changed_preview_again"
                | "file_changed"
                | "key_revoked",
            ) => "authority_changed",
            _ => "provider_or_authority_refused",
        }
    } else {
        "provider_or_authority_refused"
    }
}

async fn add_preview(value: &mut Value, guard: &Guard<'_>) -> AppResult<()> {
    let p = guard.prepared;
    // A retry reads the saved draft without loading or resending file bytes.
    let original;
    let (content_type, label) = if p.mime.is_empty() {
        original = uploads::for_egress(guard.db, &p.chat, &p.attachment).await?;
        (original.content_type(), original.label())
    } else {
        (p.mime.as_str(), p.name.as_str())
    };
    value["preview"] = json!({"company":p.company,"company_url":p.company_url,"caption":p.caption,"attachment_id":p.attachment,"image_endpoint":p.image_url(),"content_type":content_type,"label":label});
    value["instructions"] = json!(
        "Show this company, full caption and original image in this chat. Edits require preview again with draft_id. Publish with this exact draft_id, company, caption and attachment_id to request the human's publication confirmation. Never delegate this file to another thread."
    );
    Ok(())
}
async fn prepare_image(
    guard: &Guard<'_>,
    transport: &Transport<'_>,
    adapter: &dyn ImagePublisher,
    row: &mut ImagePublication,
) -> AppResult<()> {
    if row.state != "processing" && row.image.is_none() {
        if !guard.prepared.upload() {
            return Err(refused("file_transfer_not_admitted"));
        }

        row.stage = "initialization".into();
        let effect = guard.for_row(row, "preparing");
        let reply = transport
            .api(&effect, adapter, adapter.initialize(&row.company))
            .await?;
        row.http_status = Some(reply.status);
        if !(200..300).contains(&reply.status) || !reply.body_complete {
            return Err(refused(&format!("initialization_http_{}", reply.status)));
        }
        let upload = adapter.upload(&reply.body).map_err(refused)?;
        row.image = Some(upload.image.clone());
        update(guard.db, row, "preparing", "upload", None).await?;
        let effect = guard.for_row(row, "preparing");
        let status = transport.upload(&effect, adapter, &upload).await?;
        row.http_status = Some(status);
        if !(200..300).contains(&status) {
            return Err(refused(&format!("upload_http_{status}")));
        }
        row.uploaded = true;
    }
    if row.state != "processing" {
        update(guard.db, row, "processing", "readiness", None).await?;
    }
    poll_ready(guard, transport, adapter, row).await
}
async fn poll_ready(
    guard: &Guard<'_>,
    transport: &Transport<'_>,
    adapter: &dyn ImagePublisher,
    row: &mut ImagePublication,
) -> AppResult<()> {
    use nyxid_service_adapters::image_publication::Readiness;
    for attempt in 0..12 {
        let effect = guard.for_row(row, &row.state);
        let image = row
            .image
            .as_deref()
            .ok_or_else(|| refused("image_missing"))?;
        let reply = transport
            .api(&effect, adapter, adapter.image_status(image))
            .await?;
        row.http_status = Some(reply.status);
        if reply.status != 200 || !reply.body_complete {
            return Err(refused(&format!("readiness_http_{}", reply.status)));
        }
        match adapter
            .readiness(&reply.body, &row.company, image)
            .map_err(refused)?
        {
            Readiness::Ready => return Ok(()),
            Readiness::Failed => return Err(refused("image_processing_failed")),
            Readiness::Pending if attempt < 11 => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await
            }
            _ => return Err(refused("image_readiness_timeout")),
        }
    }
    Err(refused("image_readiness_timeout"))
}
async fn publish(
    guard: &Guard<'_>,
    transport: &Transport<'_>,
    adapter: &dyn ImagePublisher,
) -> AppResult<Value> {
    let p = guard.prepared;
    let mut row = load(
        guard.db,
        &p.draft
            .as_ref()
            .ok_or_else(|| refused("draft_required"))?
            .id,
    )
    .await?;
    if matches!(
        row.state.as_str(),
        "created" | "unknown" | "failed" | "dispatching"
    ) {
        return Ok(receipt(adapter, &row));
    }
    if row.state != "ready" {
        return Err(refused("draft_not_ready_or_cancelled"));
    }
    let effect = guard.for_row(&row, "ready");
    effect.revalidate().await?;
    // Approval can wait. Recheck the provider's organization-owned image before
    // crossing the final create barrier.
    let reply = transport
        .api(
            &effect,
            adapter,
            adapter.image_status(
                row.image
                    .as_deref()
                    .ok_or_else(|| refused("image_missing"))?,
            ),
        )
        .await?;
    if reply.status != 200
        || adapter
            .readiness(
                &reply.body,
                &row.company,
                row.image.as_deref().unwrap_or_default(),
            )
            .map_err(refused)?
            != nyxid_service_adapters::image_publication::Readiness::Ready
    {
        return Err(refused("image_no_longer_ready"));
    }
    // Record before the only create send, including process death between the
    // barrier and send. Such a crash is conservatively unknown, never retried.
    update(guard.db, &mut row, "dispatching", "publication", None).await?;
    let effect = guard.for_row(&row, "dispatching");
    let request = adapter.post(
        &p.company,
        &p.caption,
        row.image
            .as_deref()
            .ok_or_else(|| refused("image_missing"))?,
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(35),
        transport.api(&effect, adapter, request),
    )
    .await;
    match result {
        Ok(Ok(reply)) => {
            row.post = reply.receipt;
            row.last_code = None;
            if (200..300).contains(&reply.status) {
                row.posted_at = Some(Utc::now());
            }
            let state = if (200..300).contains(&reply.status) {
                "created"
            } else if definite_refusal(reply.status) {
                "failed"
            } else {
                "unknown"
            };
            row.verification = if row.post.is_some() {
                "pending_readback"
            } else {
                "receipt_missing"
            }
            .into();
            update(guard.db, &mut row, state, "publication", Some(reply.status)).await?;
            if state == "created" && row.post.is_some() {
                reconcile(guard, transport, adapter, &mut row).await?;
            }
        }
        Ok(Err(transport::ApiFailure::NotSent(_))) => {
            row.last_code = Some("not_sent".into());
            row.verification = "not_submitted".into();
            update(guard.db, &mut row, "ready", "publication", None).await?;
        }
        _ => {
            // The transport saves creation headers before touching the body.
            row = load(guard.db, &row.id).await?;
            let state = if row.http_status.is_some_and(|s| (200..300).contains(&s)) {
                "created"
            } else {
                "unknown"
            };
            row.verification = "outcome_unknown".into();
            let status = row.http_status;
            update(guard.db, &mut row, state, "publication", status).await?;
        }
    }
    Ok(receipt(adapter, &row))
}
async fn reconcile(
    guard: &Guard<'_>,
    transport: &Transport<'_>,
    adapter: &dyn ImagePublisher,
    row: &mut ImagePublication,
) -> AppResult<()> {
    let effect = guard.for_row(row, &row.state);
    let request = adapter.readback(
        row.post
            .as_deref()
            .ok_or_else(|| refused("receipt_missing"))?,
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        transport.api(&effect, adapter, request),
    )
    .await;
    let verification = match result {
        Ok(Ok(reply)) if reply.status == 200 && reply.body_complete => {
            if adapter
                .readback_caption(
                    &reply.body,
                    row.post.as_deref().unwrap_or_default(),
                    &row.company,
                    row.image.as_deref().unwrap_or_default(),
                )
                .is_ok_and(|wire| hash(wire.as_bytes()) == row.wire_caption_sha256)
            {
                "verified"
            } else {
                "mismatch"
            }
        }
        Ok(Ok(reply)) if matches!(reply.status, 401 | 403) => "readback_permission_unavailable",
        Ok(Ok(reply)) if reply.status == 404 => "currently_not_found",
        _ => "readback_unavailable",
    };
    row.checked_at = Some(Utc::now());
    if verification == "verified" {
        row.verified_at = row.checked_at;
        row.last_code = None;
    }
    row.verification = verification.into();
    let state = if row.verification == "verified" {
        "created".into()
    } else {
        row.state.clone()
    };
    let status = row.http_status;
    update(guard.db, row, &state, "readback", status).await
}

#[cfg(test)]
mod tests;
