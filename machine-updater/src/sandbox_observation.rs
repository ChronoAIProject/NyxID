//! Safe Kubernetes observation diagnostics. This module never retains upstream
//! errors, request URLs, credentials, resource payloads or identity material.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationStage {
    Request,
    DecodeResponse,
    ValidateList,
    ValidateItem,
    ValidateIdentity,
    Pagination,
}
impl ObservationStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::DecodeResponse => "decode_response",
            Self::ValidateList => "validate_list",
            Self::ValidateItem => "validate_item",
            Self::ValidateIdentity => "validate_identity",
            Self::Pagination => "pagination",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationResourceKind {
    Sandbox,
    Pod,
    PersistentVolumeClaim,
    PersistentVolume,
    VolumeAttachment,
}
impl ObservationResourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "Sandbox",
            Self::Pod => "Pod",
            Self::PersistentVolumeClaim => "PersistentVolumeClaim",
            Self::PersistentVolume => "PersistentVolume",
            Self::VolumeAttachment => "VolumeAttachment",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationVerb {
    Get,
    List,
    Patch,
    Delete,
}
impl ObservationVerb {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::List => "list",
            Self::Patch => "patch",
            Self::Delete => "delete",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationErrorKind {
    Authentication,
    Forbidden,
    Timeout,
    Transport,
    HttpStatus,
    InvalidResponse,
    InvalidIdentity,
    PaginationIncomplete,
    InventoryLimit,
    CredentialUnavailable,
}
impl ObservationErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Forbidden => "forbidden",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::HttpStatus => "http_status",
            Self::InvalidResponse => "invalid_response",
            Self::InvalidIdentity => "invalid_identity",
            Self::PaginationIncomplete => "pagination_incomplete",
            Self::InventoryLimit => "inventory_limit",
            Self::CredentialUnavailable => "credential_unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Kubernetes observation failed: stage={} resource_kind={} verb={} error_kind={} http_status={http_status:?}", stage.as_str(), resource_kind.as_str(), verb.as_str(), error_kind.as_str())]
pub struct SandboxObservationError {
    pub stage: ObservationStage,
    pub resource_kind: ObservationResourceKind,
    pub verb: ObservationVerb,
    pub error_kind: ObservationErrorKind,
    pub http_status: Option<u16>,
}
