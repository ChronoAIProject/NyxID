//! Deterministic in-memory Drive for local demos and adversarial tests.

use std::{collections::BTreeMap, sync::Mutex};

use async_trait::async_trait;
use http::Method;
use serde_json::json;

use crate::{
    Error, Request, Response, Transport,
    drive::{FOLDER, FileMetadata},
    parse_json,
};

#[derive(Default)]
struct State {
    files: BTreeMap<String, FileMetadata>,
    requests: Vec<Request>,
}

pub struct MockDrive {
    state: Mutex<State>,
}

impl Default for MockDrive {
    fn default() -> Self {
        let drive = Self {
            state: Mutex::new(State::default()),
        };
        for (id, name, mime, parent) in [
            ("client-a", "Client A", FOLDER, None),
            ("reports", "Reports", FOLDER, Some("client-a")),
            (
                "report",
                "October report.txt",
                "text/plain",
                Some("reports"),
            ),
            ("brief", "Brief.txt", "text/plain", Some("client-a")),
            ("payroll", "Payroll", FOLDER, None),
            ("salary", "Salaries.txt", "text/plain", Some("payroll")),
            (
                "shortcut-out",
                "Payroll shortcut",
                "application/vnd.google-apps.shortcut",
                Some("client-a"),
            ),
        ] {
            drive.put(FileMetadata {
                id: id.into(),
                name: name.into(),
                mime_type: mime.into(),
                parents: parent.map(|s| vec![s.into()]).unwrap_or_default(),
                trashed: false,
            });
        }
        drive
    }
}

impl MockDrive {
    pub fn put(&self, file: FileMetadata) {
        self.state
            .lock()
            .unwrap()
            .files
            .insert(file.id.clone(), file);
    }
    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }
    pub fn file(&self, id: &str) -> Option<FileMetadata> {
        self.state.lock().unwrap().files.get(id).cloned()
    }
}

#[async_trait]
impl Transport for MockDrive {
    async fn send(&self, request: &Request) -> Result<Response, Error> {
        let mut state = self.state.lock().unwrap();
        state.requests.push(request.clone());
        let id = request
            .path
            .strip_prefix("/drive/v3/files/")
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        let reply = |status: u16, value: serde_json::Value| {
            Ok(Response {
                status,
                content_type: "application/json".into(),
                body: serde_json::to_vec(&value).unwrap().into(),
            })
        };
        match request.method {
            Method::GET if request.path == "/drive/v3/files" => {
                let parent = request
                    .query
                    .get("q")
                    .and_then(|q| q.split('\'').nth(1))
                    .unwrap_or("");
                let size: usize = request
                    .query
                    .get("pageSize")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(100);
                let start: usize = request
                    .query
                    .get("pageToken")
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|_| Error::Upstream)?
                    .unwrap_or(0);
                let files: Vec<_> = state
                    .files
                    .values()
                    .filter(|f| !f.trashed && f.parents == [parent])
                    .collect();
                let page: Vec<_> = files.iter().skip(start).take(size).collect();
                let mut value = json!({"files": page});
                if start.saturating_add(size) < files.len() {
                    value["nextPageToken"] = json!((start + size).to_string());
                }
                reply(200, value)
            }
            Method::GET => match state.files.get(id) {
                None => reply(404, json!({"error":"not found"})),
                Some(file)
                    if request.query.get("alt").is_some_and(|s| s == "media")
                        || request.path.ends_with("/export") =>
                {
                    Ok(Response {
                        status: 200,
                        content_type: "text/plain".into(),
                        body: format!("Demo content of {}", file.name).into(),
                    })
                }
                Some(file) => reply(200, json!(file)),
            },
            Method::POST if request.path == "/drive/v3/files" => {
                let body = parse_json(&request.body)?;
                let file = FileMetadata {
                    id: format!("created-{}", state.requests.len()),
                    name: body["name"].as_str().unwrap_or("").into(),
                    mime_type: body["mimeType"].as_str().unwrap_or("").into(),
                    parents: serde_json::from_value(body["parents"].clone())
                        .map_err(|_| Error::Upstream)?,
                    trashed: false,
                };
                state.files.insert(file.id.clone(), file.clone());
                reply(200, json!(file))
            }
            Method::PATCH => {
                let body = parse_json(&request.body)?;
                let Some(file) = state.files.get_mut(id) else {
                    return reply(404, json!({"error":"not found"}));
                };
                file.name = body["name"].as_str().unwrap_or("").into();
                reply(200, json!(file))
            }
            _ => reply(400, json!({"error":"unsupported mock operation"})),
        }
    }
}
