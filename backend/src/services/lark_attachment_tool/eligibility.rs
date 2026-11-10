//! One admission predicate for discovery, preparation and managed child effects.
//! No message values or credential identifiers are included in refusals.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(crate) enum Reason {
    Guest,
    FlagOff,
    NotLive,
    KeyIneligible,
    ConnectionRequired,
    CatalogUnavailable,
    ConnectionUnavailable,
    SpecOverride,
    CatalogMismatch,
    NotPublished,
    MissingOperation,
    UntrustedContract,
    ContractChanged,
    InvalidGeneration,
    GenerationChanged,
    HostedRequired,
    ConnectionCredentialRequired,
    CredentialUnavailable,
    CredentialChanged,
    DestinationChanged,
    BindingChanged,
    AsyncUnsupported,
    LiveExecutionRequired,
}

impl Reason {
    pub(crate) fn error(self) -> AppError {
        let (reason, action) = match self {
            Self::Guest => (
                "guest_turn",
                "Only the owner may upload attachments; use an owner chat",
            ),
            Self::FlagOff => (
                "flag_off",
                "Enable assistant:attachment-writes for the acting owner",
            ),
            Self::NotLive => (
                "not_live_owner_chat",
                "Start a live, unstopped owner chat turn",
            ),
            Self::KeyIneligible => (
                "key_ineligible",
                "Use the active, unexpired key bound to this owner conversation",
            ),
            Self::ConnectionRequired => (
                "catalog_connection_required",
                "Select a catalog-backed user connection",
            ),
            Self::CatalogUnavailable => (
                "catalog_unavailable",
                "Restore the active trusted catalog service",
            ),
            Self::ConnectionUnavailable => (
                "connection_unavailable",
                "Restore the selected active connection and its endpoint",
            ),
            Self::SpecOverride => (
                "untrusted_spec_override",
                "Clear the connection OpenAPI URL override to use its trusted catalog operations",
            ),
            Self::CatalogMismatch => (
                "catalog_binding_changed",
                "Reload the selected connection; its catalog binding changed",
            ),
            Self::NotPublished => (
                "operation_not_published",
                "An administrator must publish and activate every required operation",
            ),
            Self::MissingOperation => (
                "required_operation_missing",
                "Restore the complete catalog operation set through catalog spec sync",
            ),
            Self::UntrustedContract => (
                "untrusted_contract",
                "Use the matching compiled SYSTEM Lark/Feishu catalog contracts",
            ),
            Self::ContractChanged => (
                "contract_changed",
                "Reload the tool; its trusted operation contract changed",
            ),
            Self::InvalidGeneration => (
                "invalid_operation_generation",
                "An administrator must repair the invalid catalog operation generation",
            ),
            Self::GenerationChanged => (
                "operation_generation_changed",
                "Reload the tool; its operation generation changed",
            ),
            Self::HostedRequired => (
                "hosted_connection_required",
                "Use a server-held connection credential without node routing",
            ),
            Self::ConnectionCredentialRequired => (
                "connection_credential_required",
                "Use the connection's own credential for managed Base writes",
            ),
            Self::CredentialUnavailable => (
                "credential_unavailable",
                "Reconnect the selected connection credential",
            ),
            Self::CredentialChanged => (
                "credential_changed",
                "The connection credential changed; review the saved write outcome before retrying",
            ),
            Self::DestinationChanged => (
                "destination_changed",
                "The child request no longer matches the approved destination",
            ),
            Self::BindingChanged => (
                "chat_binding_changed",
                "Use the same owner, conversation, turn and key that admitted this upload",
            ),
            Self::LiveExecutionRequired => (
                "live_chat_execution_required",
                "Use the attachment tool from a live owner chat; ordinary, exact and durable dispatch are not supported",
            ),
            Self::AsyncUnsupported => (
                "async_unsupported",
                "Attachment writes require live synchronous chat execution",
            ),
        };
        AppError::Forbidden(format!("attachment_write_{reason}: {action}"))
    }
}

pub(crate) fn managed_candidate(endpoint: &McpToolEndpoint) -> bool {
    endpoint.name == COMPOSITE_OPERATION
        || endpoint.path == COMPOSITE_PATH
        || endpoint.path == format!("/open-apis{COMPOSITE_PATH}")
}

pub(crate) async fn require_chat(db: &Database, chat: Option<&ChatAuthority>) -> AppResult<()> {
    let chat = chat.ok_or_else(|| Reason::NotLive.error())?;
    if chat.guest {
        return Err(Reason::Guest.error());
    }
    if !writes_enabled(db, Some(chat)).await? {
        return Err(Reason::FlagOff.error());
    }
    super::super::assistant_resource_service::require_live(db, chat)
        .await
        .map_err(|error| match error {
            AppError::AssistantTurnRequired | AppError::NotFound(_) | AppError::Forbidden(_) => {
                Reason::NotLive.error()
            }
            other => other,
        })?;
    if !uploads::eligible_egress_key(db, chat).await? {
        return Err(Reason::KeyIneligible.error());
    }
    Ok(())
}

pub(crate) async fn require_eligible(
    db: &Database,
    chat: Option<&ChatAuthority>,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
) -> AppResult<BTreeMap<String, String>> {
    require_chat(db, chat).await?;
    require_operation(db, service, endpoint).await
}

/// Also used by discovery after checking the request's chat once. All database
/// reads are request-local; execution rechecks before each provider effect.
pub(crate) async fn require_operation(
    db: &Database,
    service: &McpToolService,
    endpoint: &McpToolEndpoint,
) -> AppResult<BTreeMap<String, String>> {
    if endpoint.async_operation.is_some() {
        return Err(Reason::AsyncUnsupported.error());
    }
    if managed_candidate(endpoint) {
        match &service.source {
            McpToolSource::UserManaged {
                node_id,
                has_server_credential: true,
                ..
            } if node_id.as_deref().is_none_or(str::is_empty) => {}
            McpToolSource::UserManaged { .. } => return Err(Reason::HostedRequired.error()),
            _ => return Err(Reason::ConnectionRequired.error()),
        }
        super::super::lark_base_writes::contracts(db, service, endpoint).await
    } else {
        validate_contract(db, service, endpoint).await?;
        Ok(BTreeMap::new())
    }
}

pub(crate) fn is_refusal(error: &AppError) -> bool {
    matches!(error, AppError::Forbidden(message) if message.starts_with("attachment_write_"))
}
