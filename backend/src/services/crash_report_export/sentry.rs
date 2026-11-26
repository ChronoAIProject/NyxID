use super::*;
use crate::models::client_error_report::{Kind, Platform};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub struct SentryExporter {
    client: reqwest::Client,
    endpoint: url::Url,
    auth: String,
    id: String,
}
impl std::fmt::Debug for SentryExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SentryExporter([redacted])")
    }
}
impl SentryExporter {
    pub fn new(config: &crate::config::CrashDestinationConfig) -> Result<Self, &'static str> {
        let endpoint = config.endpoint.clone();
        let id = hex::encode(Sha256::digest(endpoint.as_str().as_bytes()));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .no_proxy()
            .build()
            .map_err(|_| "Cannot initialize crash transport")?;
        Ok(Self {
            client,
            endpoint,
            auth: config.auth.clone(),
            id,
        })
    }
}
/// A fresh allowlist projection, never a serialization of a stored document.
pub fn envelope(safe: &SanitizedReport<'_>) -> Vec<u8> {
    let r = safe.report;
    let event_id = uuid::Uuid::parse_str(&r.event_id)
        .expect("validated event")
        .simple()
        .to_string();
    let native = r.kind == Kind::NativeCrash;
    let frames: Vec<Value> = if native {
        r.native_frames
            .iter()
            .rev()
            .map(|f| {
                // Unknown images have no address: absolute process addresses are never exported.
                match f.image {
                    Some(index) => json!({
                        "instruction_addr": format!("0x{:x}", f.offset),
                        "addr_mode": format!("rel:{index}"),
                        "package": r.native_images[usize::from(index)].name
                    }),
                    None => json!({"instruction_addr": "0x0"}),
                }
            })
            .collect()
    } else {
        r.js_frames
            .iter()
            .rev()
            .map(|f| {
                json!({
                    "abs_path": format!("app:///{}", f.asset),
                    "lineno": f.line,
                    "colno": f.column
                })
            })
            .collect()
    };
    let release = if r.platform == Platform::Web {
        format!(
            "nyxid-web@{}+{}",
            r.app_version,
            r.artifact_id.as_deref().unwrap_or("unknown")
        )
    } else {
        format!(
            "nyxid-{}@{}+{}",
            r.platform.as_str(),
            r.app_version,
            r.app_build
        )
    };
    let mut event = json!({
        "event_id": event_id, "timestamp": crate::services::crash_report_service::validation::captured_at(
            &r.captured_at
        ).expect("validated timestamp").to_rfc3339(), "level": safe.severity,
        "platform": if native {"cocoa"} else {"javascript"},
        "release": release, "dist": r.app_build,
        "environment": safe.environment,
        "sdk": {
            "name": "nyxid.crash-export",
            "version": env!("CARGO_PKG_VERSION"),
            "settings": {"infer_ip": "never"}
        },
        "exception": {"values": [{
            "type": r.native_signal.map(|s| s.as_str()).unwrap_or(r.exception_class.as_str()),
            "mechanism": {
                "type": r.kind.as_str(),
                "handled": matches!(
                    r.kind,
                    Kind::RenderError | Kind::RecoverableError | Kind::ResourceError
                )
            },
            "stacktrace": {"frames": frames}}]},
        "contexts": {"os": {"version": r.os_version}},
        "tags": {"kind": r.kind.as_str(), "platform": r.platform.as_str()}
    });
    if r.platform != Platform::Web {
        event["contexts"]["os"]["name"] = json!(if r.platform == Platform::Ios {
            "iOS"
        } else {
            "Android"
        });
    }
    if let Some(runtime) = &r.runtime {
        event["contexts"][if r.platform == Platform::Web {
            "browser"
        } else {
            "runtime"
        }] = json!({"name": runtime.name.as_str(), "version": runtime.major.to_string()});
    }
    if native {
        event["debug_meta"] = json!({
            "images": r.native_images.iter().map(|i| json!({
                "type": "macho",
                "debug_id": i.uuid,
                "image_addr": "0x0",
                "image_size": i.size,
                "code_file": i.name
            })).collect::<Vec<_>>()
        });
    }
    let payload = serde_json::to_vec(&event).expect("closed envelope");
    let mut bytes = format!(
        "{}\n{}\n",
        json!({"event_id": event_id}),
        json!({"type": "event", "length": payload.len()})
    )
    .into_bytes();
    bytes.extend(payload);
    bytes.push(b'\n');
    bytes
}
#[async_trait::async_trait]
impl CrashReportExporter for SentryExporter {
    fn destination_id(&self) -> &str {
        &self.id
    }
    async fn submit(&self, safe: &SanitizedReport<'_>) -> SubmitOutcome {
        // Reviewed final network boundary: worker rechecks consent and persists dispatching first.
        let result = self
            .client
            .post(self.endpoint.clone())
            .header("X-Sentry-Auth", &self.auth)
            .header("Content-Type", "application/x-sentry-envelope")
            .body(envelope(safe))
            .send()
            .await;
        match result {
            Ok(response) if response.status().is_success() => SubmitOutcome::Exported,
            Ok(response) if matches!(response.status().as_u16(), 429 | 503) => {
                let after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| {
                        v.parse::<u64>().ok().or_else(|| {
                            httpdate::parse_http_date(v).ok().map(|date| {
                                date.duration_since(std::time::SystemTime::now())
                                    .unwrap_or_default()
                                    .as_secs()
                            })
                        })
                    })
                    .unwrap_or(10)
                    .clamp(1, 3600);
                SubmitOutcome::Retry {
                    after,
                    code: "provider_retry",
                }
            }
            Ok(response) if response.status().is_client_error() => SubmitOutcome::Failed,
            Err(error) if error.is_connect() => SubmitOutcome::Retry {
                after: 10,
                code: "connect_failed",
            },
            _ => SubmitOutcome::Unknown,
        }
    }
}
#[cfg(test)]
#[path = "sentry_tests.rs"]
mod tests;
