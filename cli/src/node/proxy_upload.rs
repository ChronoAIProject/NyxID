//! Bounded request-body streaming for credential-node proxying. The opening
//! metadata is always signed, independently of the legacy proxy signing switch.
use anyhow::{Context, Result, bail};
use nyxid_machine::{
    Operation, Request,
    binary::{Frame, Kind},
    signing::ReplayGuard,
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

#[derive(Default)]
pub struct Uploads {
    replay: Mutex<ReplayGuard>,
    streams: Mutex<HashMap<Uuid, Stream>>,
}
struct Stream {
    sender: mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
    sequence: u64,
}
/// Constructible only after signature verification. The executor cannot opt
/// out of authentication by accepting an untrusted JSON field.
pub struct VerifiedUpload {
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = std::io::Result<Vec<u8>>> + Send>>,
    operation: Operation,
    git: bool,
}
impl VerifiedUpload {
    pub fn git(&self) -> bool {
        self.git
    }
    pub fn into_body(self) -> reqwest::Body {
        reqwest::Body::wrap_stream(self.stream)
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn into_stream(
        self,
    ) -> std::pin::Pin<Box<dyn futures::Stream<Item = std::io::Result<Vec<u8>>> + Send>> {
        self.stream
    }
}
impl Uploads {
    pub async fn disconnect(&self) {
        // Replay memory belongs to the daemon, whereas body pipes belong to
        // one socket. Drop body senders without forgetting accepted nonces.
        self.streams.lock().await.clear();
    }
    pub async fn begin(
        self: &Arc<Self>,
        value: serde_json::Value,
        node_id: &str,
        secret: &str,
    ) -> Result<(serde_json::Value, VerifiedUpload)> {
        let request: Request = serde_json::from_value(value)?;
        if !matches!(
            request.operation,
            Operation::ProxyUpload | Operation::SaveAttachment | Operation::ShareFile
        ) {
            bail!("invalid upload operation");
        }
        let signing = zeroize::Zeroizing::new(hex::decode(secret)?);
        self.replay
            .lock()
            .await
            .verify(&request, node_id, &signing, chrono::Utc::now().timestamp())
            .map_err(|_| anyhow::anyhow!("upload signature refused"))?;
        let id = Uuid::parse_str(&request.request_id)?;
        let mut metadata = request.parameters;
        metadata["_authority"] = serde_json::to_value(&request.authority)?;
        metadata["_authority_version"] = request.version.into();
        let git = metadata["git"].as_bool().unwrap_or(false);
        let limit = metadata["max_bytes"]
            .as_u64()
            .context("missing upload limit")?;
        if limit > 16 * 1024 * 1024 * 1024 || metadata["body"].as_str().is_some() {
            bail!("invalid upload limit or inline body");
        }
        if git
            && (metadata["base_url"] != "https://github.com"
                || !matches!(
                    metadata["service_slug"].as_str(),
                    Some("api-github" | "api-github-pat")
                ))
        {
            bail!("invalid git destination");
        }
        metadata["request_id"] = request.request_id.into();
        let (tx, mut rx) = mpsc::channel(16);
        {
            let mut streams = self.streams.lock().await;
            if streams.len() >= 32 || streams.contains_key(&id) {
                bail!("upload concurrency limit");
            }
            streams.insert(
                id,
                Stream {
                    sender: tx,
                    sequence: 0,
                },
            );
        }
        let guard = Cleanup {
            uploads: Arc::downgrade(self),
            id,
        };
        let stream = async_stream::try_stream! {
            let _guard=guard;
            let mut total=0u64;
            loop {
                let bytes=tokio::time::timeout(Duration::from_secs(60),rx.recv()).await
                    .map_err(|_|std::io::Error::other("upload idle timeout"))?
                    .ok_or_else(||std::io::Error::other("upload disconnected"))??;
                if bytes.is_empty(){break;}
                total=total.saturating_add(bytes.len() as u64);
                if total>limit {Err(std::io::Error::other("upload limit exceeded"))?;}
                yield bytes;
            }
        };
        Ok((
            metadata,
            VerifiedUpload {
                stream: Box::pin(futures::StreamExt::map(
                    stream,
                    |item: Result<Vec<u8>, std::io::Error>| item,
                )),
                operation: request.operation,
                git,
            },
        ))
    }
    pub async fn frame(&self, frame: Frame<'_>) {
        let (sender, valid) = {
            let mut streams = self.streams.lock().await;
            let Some(stream) = streams.get_mut(&frame.id) else {
                return;
            };
            let valid = frame.kind == Kind::ProxyUpload && frame.sequence == stream.sequence;
            stream.sequence += 1;
            let sender = stream.sender.clone();
            if frame.end || !valid {
                streams.remove(&frame.id);
            }
            (sender, valid)
        };
        if !valid {
            let _ = sender.try_send(Err(std::io::Error::other("upload interrupted")));
            return;
        }
        if !frame.bytes.is_empty()
            && !tokio::time::timeout(
                Duration::from_secs(1),
                sender.send(Ok(frame.bytes.to_vec())),
            )
            .await
            .is_ok_and(|result| result.is_ok())
        {
            self.streams.lock().await.remove(&frame.id);
            return;
        }
        if frame.end {
            let _ = tokio::time::timeout(Duration::from_secs(1), sender.send(Ok(Vec::new()))).await;
        }
    }
}
struct Cleanup {
    uploads: std::sync::Weak<Uploads>,
    id: Uuid,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let uploads = self.uploads.clone();
        let id = self.id;
        tokio::spawn(async move {
            if let Some(uploads) = uploads.upgrade() {
                uploads.streams.lock().await.remove(&id);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use serde_json::json;
    fn opening() -> Request {
        let mut request = Request {
            version: 1,
            authority: None,
            request_id: Uuid::new_v4().to_string(),
            node_id: "credential-node".into(),
            operation: Operation::ProxyUpload,
            parameters: json!({"method":"POST","base_url":"https://example.test","service_slug":"connected","headers":{},"max_bytes":65536}),
            timestamp: chrono::Utc::now().timestamp(),
            nonce: Uuid::new_v4().to_string(),
            signature: String::new(),
        };
        request.signature = nyxid_machine::signing::sign(&request, &[1; 32]);
        request
    }
    #[tokio::test]
    async fn upload_requires_signed_metadata_and_rejects_tampering_and_replay() {
        let uploads = Arc::new(Uploads::default());
        let request = opening();
        let (_, body) = uploads
            .begin(
                serde_json::to_value(&request).unwrap(),
                "credential-node",
                &hex::encode([1; 32]),
            )
            .await
            .unwrap();
        assert!(
            uploads
                .begin(
                    serde_json::to_value(&request).unwrap(),
                    "credential-node",
                    &hex::encode([1; 32])
                )
                .await
                .is_err()
        );
        drop(body);
        for mutate in 0..4 {
            let mut request = opening();
            match mutate {
                0 => request.parameters["base_url"] = json!("https://attacker.test"),
                1 => request.signature.clear(),
                2 => request.node_id = "another-node".into(),
                _ => {
                    request.timestamp -= 61;
                    request.signature = nyxid_machine::signing::sign(&request, &[1; 32]);
                }
            }
            assert!(
                uploads
                    .begin(
                        serde_json::to_value(request).unwrap(),
                        "credential-node",
                        &hex::encode([1; 32])
                    )
                    .await
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn upload_chunks_are_bounded_ordered_and_explicitly_terminated() {
        let uploads = Arc::new(Uploads::default());
        let request = opening();
        let id = Uuid::parse_str(&request.request_id).unwrap();
        let (_, upload) = uploads
            .begin(
                serde_json::to_value(request).unwrap(),
                "credential-node",
                &hex::encode([1; 32]),
            )
            .await
            .unwrap();
        let mut body = axum::body::Body::new(upload.into_body()).into_data_stream();
        uploads
            .frame(Frame {
                kind: Kind::ProxyUpload,
                id,
                sequence: 0,
                end: false,
                bytes: b"",
            })
            .await;
        uploads
            .frame(Frame {
                kind: Kind::ProxyUpload,
                id,
                sequence: 1,
                end: false,
                bytes: b"first",
            })
            .await;
        assert_eq!(body.next().await.unwrap().unwrap(), "first");
        uploads
            .frame(Frame {
                kind: Kind::ProxyUpload,
                id,
                sequence: 2,
                end: true,
                bytes: b"last",
            })
            .await;
        assert_eq!(body.next().await.unwrap().unwrap(), "last");
        assert!(body.next().await.is_none());
        assert!(uploads.streams.lock().await.is_empty());
        let request = opening();
        let id = Uuid::parse_str(&request.request_id).unwrap();
        let (_, upload) = uploads
            .begin(
                serde_json::to_value(request).unwrap(),
                "credential-node",
                &hex::encode([1; 32]),
            )
            .await
            .unwrap();
        let mut body = axum::body::Body::new(upload.into_body()).into_data_stream();
        uploads
            .frame(Frame {
                kind: Kind::ProxyUpload,
                id,
                sequence: 1,
                end: false,
                bytes: b"wrong order",
            })
            .await;
        assert!(body.next().await.unwrap().is_err());
    }
}
