use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};

use super::{AppResult, refused};

pub const EMPTY_BODY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadDescriptor {
    pub version: u8,
    pub method: String,
    pub path_and_query: String,
    pub content_type: Option<String>,
    pub body_sha256: String,
    pub idempotency_key_sha256: Option<String>,
}

#[derive(Default, Deserialize)]
pub struct IntrospectionInput {
    pub request_method: Option<String>,
    pub request_path_and_query: Option<String>,
    pub request_content_type: Option<String>,
    pub request_body_sha256: Option<String>,
}

impl IntrospectionInput {
    pub fn descriptor(self) -> AppResult<ReadDescriptor> {
        let descriptor = ReadDescriptor {
            version: 1,
            method: self.request_method.ok_or_else(refused)?,
            path_and_query: self.request_path_and_query.ok_or_else(refused)?,
            content_type: self.request_content_type,
            body_sha256: self.request_body_sha256.ok_or_else(refused)?,
            idempotency_key_sha256: None,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}

impl ReadDescriptor {
    pub fn validate(&self) -> AppResult<()> {
        if self.version != 1
            || !matches!(self.method.as_str(), "GET" | "HEAD")
            || self.body_sha256 != EMPTY_BODY_SHA256
            || self.idempotency_key_sha256.is_some()
            || !self.path_and_query.starts_with('/')
            || self.path_and_query.starts_with("//")
            || self.path_and_query.contains('#')
            || self.path_and_query.chars().any(char::is_control)
            || self.path_and_query.len() > 8192
            || self.content_type.as_ref().is_some_and(|value| {
                value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
            })
        {
            return Err(refused());
        }
        let uri: axum::http::Uri = self.path_and_query.parse().map_err(|_| refused())?;
        if uri.scheme().is_some() || uri.authority().is_some() {
            return Err(refused());
        }
        if super::super::mcp_service::http_carries_method_override(
            &self.method,
            &HeaderMap::new(),
            uri.query(),
            &[],
        ) {
            return Err(refused());
        }
        Ok(())
    }

    pub fn from_request(request: &reqwest::Request) -> AppResult<Self> {
        let headers = request.headers();
        let content_types: Vec<_> = headers.get_all("content-type").iter().collect();
        if content_types.len() > 1
            || headers.contains_key("idempotency-key")
            || headers
                .get_all("content-encoding")
                .iter()
                .any(|value| value != "identity")
            || request
                .body()
                .is_some_and(|body| body.as_bytes() != Some(&[][..]))
            || super::super::mcp_service::http_carries_method_override(
                request.method().as_str(),
                headers,
                request.url().query(),
                &[],
            )
        {
            return Err(refused());
        }
        let descriptor = Self {
            version: 1,
            method: request.method().as_str().to_owned(),
            path_and_query: match request.url().query() {
                Some(query) => format!("{}?{query}", request.url().path()),
                None => request.url().path().to_owned(),
            },
            content_type: content_types
                .first()
                .map(|value| value.to_str().map(str::to_owned))
                .transpose()
                .map_err(|_| refused())?,
            body_sha256: EMPTY_BODY_SHA256.into(),
            idempotency_key_sha256: None,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}
