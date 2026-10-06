use std::collections::HashMap;

use async_trait::async_trait;
use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Error, Plan, Policy, ProviderAdapter, Request, ResourceBoundary, ResourceCheck, Response,
    Transport, parse_json,
};

pub const FOLDER: &str = "application/vnd.google-apps.folder";
const SHORTCUT: &str = "application/vnd.google-apps.shortcut";
const VERIFY_FIELDS: &str = "id,name,mimeType,parents,trashed";
pub const OPERATIONS: &[&str] = &[
    "drive.files.list",
    "drive.files.get",
    "drive.files.download",
    "drive.files.export",
    "drive.folders.create",
    "drive.files.rename",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    pub id: String,
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(default)]
    pub parents: Vec<String>,
    pub trashed: bool,
}

pub struct DriveAdapter;

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn reject(message: &'static str) -> Error {
    Error::Denied(message)
}

fn only_query(request: &Request, allowed: &[&str]) -> Result<(), Error> {
    if request
        .query
        .keys()
        .any(|key| !allowed.contains(&key.as_str()))
    {
        return Err(reject("query parameter is not supported"));
    }
    if request
        .query
        .get("supportsAllDrives")
        .is_some_and(|s| s != "true")
    {
        return Err(reject("supportsAllDrives must be true"));
    }
    Ok(())
}

fn json_body(request: &Request, allowed: &[&str]) -> Result<Value, Error> {
    if request
        .content_type
        .as_deref()
        .and_then(|s| s.split(';').next())
        .map(str::trim)
        != Some("application/json")
    {
        return Err(reject("this operation requires application/json"));
    }
    let body = parse_json(&request.body)?;
    let object = body
        .as_object()
        .ok_or_else(|| reject("expected a JSON object"))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(reject("body field is not supported"));
    }
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| reject("name is required"))?;
    if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err(reject("invalid file name"));
    }
    Ok(body)
}

fn checked_id(id: &str, folder: bool, allow_root: bool) -> Result<ResourceCheck, Error> {
    if !valid_id(id) {
        return Err(reject("invalid resource ID"));
    }
    Ok(ResourceCheck {
        id: id.to_owned(),
        must_be_folder: folder,
        allow_root,
    })
}

#[async_trait]
impl ProviderAdapter for DriveAdapter {
    fn validate_policy(&self, policy: &Policy) -> Result<(), Error> {
        let ResourceBoundary::GoogleDriveFolder { root_folder_id, .. } = &policy.resource else {
            return Err(Error::Policy("expected a Drive folder boundary"));
        };
        if policy.provider != "google_drive"
            || !valid_id(root_folder_id)
            || root_folder_id == "root"
        {
            return Err(Error::Policy(
                "expected google_drive and a concrete folder ID (not root)",
            ));
        }
        if policy
            .allowed_operations
            .iter()
            .any(|op| !OPERATIONS.contains(&op.as_str()))
        {
            return Err(Error::Policy("unknown Drive operation"));
        }
        for (operation, constraints) in &policy.constraints {
            let allowed_query: &[&str] = match operation.as_str() {
                "drive.files.list" => &[
                    "q",
                    "pageSize",
                    "pageToken",
                    "fields",
                    "supportsAllDrives",
                    "includeItemsFromAllDrives",
                ],
                "drive.files.download" => &["alt", "supportsAllDrives"],
                "drive.files.export" => &["mimeType"],
                _ => &["fields", "supportsAllDrives"],
            };
            for constraint in constraints {
                let known = match constraint.location {
                    crate::ParameterLocation::Query => {
                        allowed_query.contains(&constraint.name.as_str())
                    }
                    crate::ParameterLocation::Body => match operation.as_str() {
                        "drive.files.rename" => ["", "/name"].contains(&constraint.name.as_str()),
                        "drive.folders.create" => {
                            ["", "/name", "/mimeType", "/parents", "/parents/0"]
                                .contains(&constraint.name.as_str())
                        }
                        _ => false,
                    },
                };
                if !known {
                    return Err(Error::Policy("constraint names an unsupported parameter"));
                }
            }
        }
        Ok(())
    }

    fn plan(&self, policy: &Policy, mut request: Request) -> Result<Plan, Error> {
        let ResourceBoundary::GoogleDriveFolder { root_folder_id, .. } = &policy.resource else {
            return Err(Error::Policy("expected a Drive folder boundary"));
        };
        if request.path.len() > 2048
            || !request.path.starts_with('/')
            || request.path.contains(['%', '\\', '?', '#'])
        {
            return Err(reject("noncanonical path"));
        }
        if request.method == Method::GET && !request.body.is_empty() {
            return Err(reject("GET bodies are not supported"));
        }
        let path = request.path.clone();
        let segments: Vec<_> = path.split('/').collect();
        let mut resources = Vec::new();
        let operation = match (request.method.as_str(), segments.as_slice()) {
            ("GET", ["", "drive", "v3", "files"]) => {
                only_query(
                    &request,
                    &["q", "pageSize", "pageToken", "fields", "supportsAllDrives"],
                )?;
                // Accept a small, complete query grammar. Arbitrary boolean
                // expressions and full-text searches cannot widen containment.
                let parent = match request.query.get("q") {
                    None => root_folder_id.clone(),
                    Some(q) => q
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix("' in parents"))
                        .filter(|id| valid_id(id))
                        .map(str::to_owned)
                        .ok_or_else(|| reject("list q must be exactly 'FOLDER_ID' in parents"))?,
                };
                resources.push(checked_id(&parent, true, true)?);
                let size = request
                    .query
                    .get("pageSize")
                    .map(|s| s.parse::<u16>())
                    .transpose()
                    .map_err(|_| reject("invalid pageSize"))?
                    .unwrap_or(100);
                if !(1..=100).contains(&size) {
                    return Err(reject("pageSize must be between 1 and 100"));
                }
                if request
                    .query
                    .get("pageToken")
                    .is_some_and(|s| s.is_empty() || s.len() > 4096)
                {
                    return Err(reject("invalid page token"));
                }
                request.query.insert(
                    "q".into(),
                    format!("'{parent}' in parents and trashed = false"),
                );
                request.query.insert("pageSize".into(), size.to_string());
                request.query.insert(
                    "fields".into(),
                    format!("nextPageToken,incompleteSearch,files({VERIFY_FIELDS})"),
                );
                request
                    .query
                    .insert("includeItemsFromAllDrives".into(), "true".into());
                "drive.files.list"
            }
            ("GET", ["", "drive", "v3", "files", id]) => {
                only_query(&request, &["fields", "alt", "supportsAllDrives"])?;
                resources.push(checked_id(id, false, true)?);
                match request.query.get("alt").map(String::as_str) {
                    None | Some("json") => {
                        request.query.remove("alt");
                        request.query.insert("fields".into(), VERIFY_FIELDS.into());
                        "drive.files.get"
                    }
                    Some("media") => {
                        request.query.remove("fields");
                        "drive.files.download"
                    }
                    _ => return Err(reject("unsupported alt value")),
                }
            }
            ("GET", ["", "drive", "v3", "files", id, "export"]) => {
                only_query(&request, &["mimeType"])?;
                if !matches!(
                    request.query.get("mimeType").map(String::as_str),
                    Some("application/pdf" | "text/plain" | "text/csv")
                ) {
                    return Err(reject("unsupported export MIME type"));
                }
                resources.push(checked_id(id, false, false)?);
                "drive.files.export"
            }
            ("POST", ["", "drive", "v3", "files"]) => {
                only_query(&request, &["fields", "supportsAllDrives"])?;
                let body = json_body(&request, &["name", "mimeType", "parents"])?;
                if body["mimeType"] != FOLDER {
                    return Err(reject("only folder creation is supported"));
                }
                let parents = body["parents"]
                    .as_array()
                    .filter(|p| p.len() == 1)
                    .ok_or_else(|| reject("exactly one explicit parent is required"))?;
                let parent = parents[0]
                    .as_str()
                    .ok_or_else(|| reject("invalid parent"))?;
                resources.push(checked_id(parent, true, true)?);
                request.query.insert("fields".into(), VERIFY_FIELDS.into());
                "drive.folders.create"
            }
            ("PATCH", ["", "drive", "v3", "files", id]) => {
                only_query(&request, &["fields", "supportsAllDrives"])?;
                json_body(&request, &["name"])?;
                resources.push(checked_id(id, false, false)?);
                request.query.insert("fields".into(), VERIFY_FIELDS.into());
                "drive.files.rename"
            }
            _ => return Err(reject("operation is not supported by the Drive adapter")),
        };
        if operation != "drive.files.export" {
            request
                .query
                .insert("supportsAllDrives".into(), "true".into());
        }
        Ok(Plan {
            operation: operation.into(),
            origin: None,
            request,
            resources,
        })
    }

    async fn verify(
        &self,
        policy: &Policy,
        plan: &Plan,
        transport: &dyn Transport,
    ) -> Result<(), Error> {
        let ResourceBoundary::GoogleDriveFolder {
            root_folder_id,
            include_descendants,
        } = &policy.resource
        else {
            return Err(Error::Policy("expected a Drive folder boundary"));
        };
        let mut cache = HashMap::<String, FileMetadata>::new();
        for resource in &plan.resources {
            if resource.must_be_folder && !include_descendants && resource.id != *root_folder_id {
                return Err(reject("listing or creating inside subfolders is denied"));
            }
            if resource.id == *root_folder_id && !resource.allow_root {
                return Err(reject("changing the boundary folder itself is denied"));
            }
            let mut current = resource.id.clone();
            let mut visited = std::collections::HashSet::new();
            let mut found = false;
            for depth in 0..32 {
                if !visited.insert(current.clone()) {
                    return Err(reject("cyclic folder ancestry"));
                }
                if !cache.contains_key(&current) {
                    let mut request = Request::get(format!("/drive/v3/files/{current}"));
                    request.query.insert("fields".into(), VERIFY_FIELDS.into());
                    request
                        .query
                        .insert("supportsAllDrives".into(), "true".into());
                    let response = transport
                        .send(&request)
                        .await
                        .map_err(|_| Error::Verification)?;
                    if response.status != 200 || response.body.len() > 64 * 1024 {
                        return Err(Error::Verification);
                    }
                    let file: FileMetadata = serde_json::from_value(
                        parse_json(&response.body).map_err(|_| Error::Verification)?,
                    )
                    .map_err(|_| Error::Verification)?;
                    if file.id != current
                        || file.parents.iter().any(|id| !valid_id(id))
                        || file.parents.len() > 1
                    {
                        return Err(Error::Verification);
                    }
                    cache.insert(current.clone(), file);
                }
                let file = &cache[&current];
                if file.trashed || file.mime_type == SHORTCUT {
                    return Err(reject("trashed files and shortcuts are denied"));
                }
                if (depth > 0 || resource.must_be_folder || current == *root_folder_id)
                    && file.mime_type != FOLDER
                {
                    return Err(reject("expected a folder in resource ancestry"));
                }
                if depth == 0
                    && matches!(
                        plan.operation.as_str(),
                        "drive.files.download" | "drive.files.export"
                    )
                    && file.mime_type == FOLDER
                {
                    return Err(reject("folder content downloads are unsupported"));
                }
                if current == *root_folder_id {
                    found = true;
                    break;
                }
                if !include_descendants && depth >= 1 {
                    break;
                }
                let Some(parent) = file.parents.first() else {
                    break;
                };
                current = parent.clone();
            }
            if !found {
                return Err(reject(
                    "resource is outside the permitted folder boundary or depth limit",
                ));
            }
        }
        Ok(())
    }

    fn sanitize_response(&self, plan: &Plan, response: Response) -> Result<Response, Error> {
        if matches!(
            plan.operation.as_str(),
            "drive.files.download" | "drive.files.export"
        ) {
            return Ok(Response {
                content_type: "application/octet-stream".into(),
                ..response
            });
        }
        let value = parse_json(&response.body).map_err(|_| Error::Verification)?;
        let sanitize = |value: Value| -> Result<Value, Error> {
            let file: FileMetadata =
                serde_json::from_value(value).map_err(|_| Error::Verification)?;
            if !valid_id(&file.id)
                || file.trashed
                || file.parents.len() > 1
                || file.parents.iter().any(|id| !valid_id(id))
            {
                return Err(Error::Verification);
            }
            // Return only the metadata this POC promises. In particular, no
            // shortcut targets, permissions, download URLs or ancestor IDs.
            Ok(json!({"id":file.id, "name":file.name, "mimeType":file.mime_type}))
        };
        let result = if plan.operation == "drive.files.list" {
            if value
                .get("incompleteSearch")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Err(Error::Verification);
            }
            let files = value["files"].as_array().ok_or(Error::Verification)?;
            let limit: usize = plan.request.query["pageSize"]
                .parse()
                .map_err(|_| Error::Verification)?;
            if files.len() > limit {
                return Err(Error::Verification);
            }
            let parent = &plan.resources[0].id;
            let mut safe = Vec::new();
            for file in files {
                if file["parents"] != json!([parent]) {
                    return Err(Error::Verification);
                }
                // Shortcuts are not traversable; omit them even from discovery.
                if file["mimeType"] == SHORTCUT {
                    continue;
                }
                safe.push(sanitize(file.clone())?);
            }
            let mut result = json!({"files": safe});
            if let Some(token) = value.get("nextPageToken") {
                if token
                    .as_str()
                    .is_none_or(|s| s.is_empty() || s.len() > 4096)
                {
                    return Err(Error::Verification);
                }
                result["nextPageToken"] = token.clone();
            }
            result
        } else {
            if plan.operation != "drive.folders.create" && value["id"] != plan.resources[0].id {
                return Err(Error::Verification);
            }
            if plan.operation == "drive.folders.create"
                && (value["parents"] != json!([plan.resources[0].id])
                    || value["mimeType"] != FOLDER)
            {
                return Err(Error::Verification);
            }
            sanitize(value)?
        };
        Ok(Response {
            status: response.status,
            content_type: "application/json".into(),
            body: serde_json::to_vec(&result)
                .map_err(|_| Error::Verification)?
                .into(),
        })
    }
}

pub fn example_policy() -> Policy {
    serde_json::from_str(include_str!("../examples/drive-policy.json")).expect("bundled policy")
}
