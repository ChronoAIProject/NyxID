//! File bodies share the signed, streaming node transport with proxy uploads.
//! Only the unprivileged worker opens paths; the supervisor scrubs read output.
use super::{FileRequest, Runtime, files::Roots, string};
use crate::node::{proxy_upload::VerifiedUpload, ws_client::NodeWsMessage};
use anyhow::{Context, Result, bail};
use futures::StreamExt;
use nyxid_machine::Operation;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use zeroize::Zeroizing;

const LIMIT: u64 = 5 * 1024 * 1024;
const UPLOAD_LIMIT: u64 = nyxid_machine::MAX_ATTACHMENT_UPLOAD_BYTES as u64;

fn transfer_limit(operation: Operation, metadata: &Value) -> Result<u64> {
    let ceiling = match operation {
        Operation::SaveAttachment => UPLOAD_LIMIT,
        Operation::ShareFile => LIMIT,
        _ => bail!("invalid file operation"),
    };
    metadata["max_bytes"]
        .as_u64()
        .filter(|size| *size <= ceiling)
        .context("file transfer limit exceeded")
}

/// Closing the pipes cancels the worker; cleanup/reaping never delays takeover.
struct Worker(Option<tokio::process::Child>);
impl std::ops::Deref for Worker {
    type Target = tokio::process::Child;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("live file worker")
    }
}
impl std::ops::DerefMut for Worker {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("live file worker")
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // The dropped input/output pipes interrupt streaming and allow
            // atomic-write cleanup. Bound that grace period off the control
            // path, then kill a worker stuck in filesystem I/O.
            child.stdin.take();
            child.stdout.take();
            tokio::spawn(async move {
                if tokio::time::timeout(Duration::from_millis(50), child.wait())
                    .await
                    .is_err()
                {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            });
        }
    }
}

pub async fn execute(
    runtime: Option<Arc<Runtime>>,
    metadata: Value,
    upload: VerifiedUpload,
    sender: mpsc::Sender<NodeWsMessage>,
) {
    let id = metadata["request_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        let runtime = runtime.context("machine files disabled")?;
        runtime.transfer(&metadata, upload, &sender).await
    })
    .await;
    if !matches!(result, Ok(Ok(()))) {
        let reason = match &result {
            Ok(Err(error)) => match error.downcast_ref::<super::MachineError>() {
                Some(super::MachineError::OwnerInControl) => "owner_in_control",
                Some(super::MachineError::AuthorityStale) => "machine_authority_stale",
                Some(super::MachineError::TurnStopped) => "machine_turn_stopped",
                _ => "machine_transfer_refused",
            },
            _ => "machine_transfer_refused",
        };
        let _ = sender
            .send(NodeWsMessage::Text(
                json!({"type":"proxy_error","request_id":id,"status":403,"error":reason,"reason":reason,"retryable":true})
                    .to_string(),
            ))
            .await;
    }
}

impl Runtime {
    /// cua runs as the browser user. Never let it open an agent-supplied path
    /// with that user's access to the protected profile and display files.
    pub(super) async fn stage_clipboard_file(&self, path: &str) -> Result<tempfile::NamedTempFile> {
        tokio::time::timeout(Duration::from_secs(30), async {
            let request = FileRequest {
                roots: self.config.roots.clone(),
                excluded: self.excluded.clone(),
                operation: Operation::ShareFile,
                parameters: json!({"path":path,"max_bytes":LIMIT}),
            };
            let header = Zeroizing::new(serde_json::to_vec(&request)?);
            let mut command = tokio::process::Command::new(std::env::current_exe()?);
            self.identity.prepare_agent(&mut command)?;
            command
                .args(["node", "machine-transfer-worker"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            let mut child = Worker(Some(command.spawn()?));
            let mut input = child.stdin.take().context("file worker unavailable")?;
            input.write_u32_le(header.len() as u32).await?;
            input.write_all(&header).await?;
            input.shutdown().await?;
            drop(input);
            let suffix = std::path::Path::new(path)
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| s.len() <= 16 && s.bytes().all(|c| c.is_ascii_alphanumeric()))
                .map(|s| format!(".{s}"))
                .unwrap_or_default();
            let file = tempfile::Builder::new()
                .prefix("nyxid-clipboard-")
                .suffix(&suffix)
                .tempfile()?;
            let mut target = tokio::fs::File::from_std(file.reopen()?);
            let mut output = child.stdout.take().context("file worker unavailable")?;
            let mut buffer = Zeroizing::new(vec![0; nyxid_machine::STREAM_CHUNK_BYTES]);
            let mut pending = Zeroizing::new(Vec::new());
            let mut size = 0u64;
            loop {
                let count = output.read(&mut buffer).await?;
                size += count as u64;
                if size > LIMIT {
                    bail!("clipboard file limit exceeded");
                }
                pending.extend_from_slice(&buffer[..count]);
                let clean = self.redactor.lock().await.stream(&mut pending, count == 0);
                target.write_all(&clean).await?;
                if count == 0 {
                    break;
                }
            }
            if !child.wait().await?.success() {
                bail!("clipboard file is outside the agent's workspace or unreadable");
            }
            target.flush().await?;
            let browser = super::process::Identity::resolve(self.config.browser_user.as_deref())?;
            super::browser::chown(file.path(), browser.uid, browser.gid)?;
            Ok(file)
        })
        .await?
    }

    async fn transfer(
        self: &Arc<Self>,
        metadata: &Value,
        upload: VerifiedUpload,
        sender: &mpsc::Sender<NodeWsMessage>,
    ) -> Result<()> {
        self.ensure_authority_watch();
        let authority: Option<Box<nyxid_machine::authority::Authority>> =
            serde_json::from_value(metadata["_authority"].clone())?;
        let mut authority_stop = if let Some(authority) = &authority {
            if !authority.capabilities.allows(upload.operation(), metadata)
                || metadata["conversation_id"] != authority.conversation_id
                || metadata["turn_id"] != authority.turn_id
            {
                return Err(super::MachineError::AuthorityStale.into());
            }
            Some(
                self.authority
                    .admit(authority, &self.runtime_id, None)
                    .map_err(|_| super::MachineError::AuthorityStale)?,
            )
        } else {
            if self.authority.enrolled() {
                return Err(super::MachineError::AuthorityStale.into());
            }
            None
        };
        let _admission = self.operation_admission.read().await;
        if self.upgrading.load(std::sync::atomic::Ordering::Acquire) {
            return Err(super::MachineError::TurnStopped.into());
        }
        let scope: super::cancellation::Scope = serde_json::from_value(metadata.clone())?;
        let mut stopped = self.turns.subscribe(&scope);
        if *stopped.borrow_and_update() {
            return Err(super::MachineError::TurnStopped.into());
        }
        let mut control = self.owner_control.subscribe();
        let mut dev_control = self.dev_owner_control.subscribe();
        if *control.borrow_and_update() & 1 != 0 || *dev_control.borrow_and_update() & 1 != 0 {
            return Err(super::MachineError::OwnerInControl.into());
        }
        let result = tokio::select! {
            biased;
            _ = async { if let Some(stop)=&mut authority_stop {let _=stop.changed().await;} else {std::future::pending::<()>().await;} } => Err(super::MachineError::AuthorityStale.into()),
            _ = stopped.changed() => Err(super::MachineError::TurnStopped.into()),
            _ = control.changed() => Err(super::MachineError::OwnerInControl.into()),
            _ = dev_control.changed() => Err(super::MachineError::OwnerInControl.into()),
            result = self.transfer_inner(metadata, upload, sender) => result,
        };
        if let Some(authority) = authority {
            if !self.authority.live(&authority.lease_id) {
                return Err(super::MachineError::AuthorityStale.into());
            }
            self.authority.finish(&authority.lease_id);
        }
        result
    }

    async fn transfer_inner(
        &self,
        metadata: &Value,
        upload: VerifiedUpload,
        sender: &mpsc::Sender<NodeWsMessage>,
    ) -> Result<()> {
        if !self.config.files {
            bail!("machine files disabled");
        }
        let operation = upload.operation();
        if !matches!(operation, Operation::SaveAttachment | Operation::ShareFile) {
            bail!("invalid file operation");
        }
        let limit = transfer_limit(operation, metadata)?;
        let id = string(metadata, "request_id")?;
        let request = FileRequest {
            roots: self.config.roots.clone(),
            excluded: self.excluded.clone(),
            operation,
            parameters: metadata.clone(),
        };
        let header = Zeroizing::new(serde_json::to_vec(&request)?);
        if header.len() > 65536 {
            bail!("file metadata limit exceeded");
        }
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        self.identity.prepare_agent(&mut command)?;
        command
            .args(["node", "machine-transfer-worker"])
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = Worker(Some(command.spawn()?));
        let mut input = child
            .stdin
            .take()
            .context("file worker input unavailable")?;
        let mut output = child
            .stdout
            .take()
            .context("file worker output unavailable")?;
        input.write_u32_le(header.len() as u32).await?;
        input.write_all(&header).await?;
        sender.send(NodeWsMessage::Text(json!({"type":"proxy_response_start","request_id":id,"status":200,"headers":{"content-type":"application/octet-stream"}}).to_string())).await?;
        let upload = async {
            let mut stream = upload.into_stream();
            let mut size = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                size += chunk.len() as u64;
                if size > limit || (operation == Operation::ShareFile && size != 0) {
                    bail!("file upload limit exceeded");
                }
                input.write_all(&chunk).await?;
            }
            input.shutdown().await?;
            drop(input);
            Ok::<(), anyhow::Error>(())
        };
        let download = async {
            let mut chunk = Zeroizing::new(vec![0; nyxid_machine::STREAM_CHUNK_BYTES]);
            let mut pending = Zeroizing::new(Vec::new());
            let mut size = 0u64;
            loop {
                let length = output.read(&mut chunk).await?;
                size += length as u64;
                if size > limit.max(1024) {
                    bail!("file output limit exceeded");
                }
                pending.extend_from_slice(&chunk[..length]);
                let clean = self.redactor.lock().await.stream(&mut pending, length == 0);
                for bytes in clean.chunks(nyxid_machine::STREAM_CHUNK_BYTES) {
                    let mut frame = Vec::with_capacity(36 + bytes.len());
                    frame.extend_from_slice(id.as_bytes());
                    frame.extend_from_slice(bytes);
                    sender.send(NodeWsMessage::Binary(frame)).await?;
                }
                if length == 0 {
                    break;
                }
            }
            Ok::<(), anyhow::Error>(())
        };
        tokio::try_join!(upload, download)?;
        if !child.wait().await?.success() {
            bail!("file worker refused transfer");
        }
        sender
            .send(NodeWsMessage::Text(
                json!({"type":"proxy_response_end","request_id":id}).to_string(),
            ))
            .await?;
        Ok(())
    }
}

pub fn worker() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut length = [0; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 65536 {
        bail!("file metadata limit exceeded");
    }
    let mut header = Zeroizing::new(vec![0; length]);
    input.read_exact(&mut header)?;
    let request: FileRequest = serde_json::from_slice(&header)?;
    let roots = Roots::new(&request.roots, &request.excluded)?;
    let path = string(&request.parameters, "path")?;
    let limit = transfer_limit(request.operation, &request.parameters)?;
    let mut output = std::io::stdout().lock();
    match request.operation {
        Operation::SaveAttachment => {
            let length = request.parameters["size"]
                .as_u64()
                .filter(|n| *n <= limit)
                .context("invalid transfer size")?;
            let hash = string(&request.parameters, "sha256")?;
            let sha256 =
                roots.write_stream(path, &mut input, length, "create", None, Some(hash))?;
            serde_json::to_writer(&mut output, &json!({"sha256":sha256,"bytes":length}))?;
        }
        Operation::ShareFile => roots.stream_read(path, &mut output, limit)?,
        _ => bail!("invalid file transfer operation"),
    }
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_attachment_limit_does_not_widen_image_or_clipboard_reads() {
        let large = json!({"max_bytes": UPLOAD_LIMIT});
        assert_eq!(
            transfer_limit(Operation::SaveAttachment, &large).unwrap(),
            UPLOAD_LIMIT
        );
        assert!(
            transfer_limit(
                Operation::SaveAttachment,
                &json!({"max_bytes": UPLOAD_LIMIT + 1})
            )
            .is_err()
        );
        assert!(transfer_limit(Operation::ShareFile, &large).is_err());
        assert_eq!(
            transfer_limit(Operation::ShareFile, &json!({"max_bytes": LIMIT})).unwrap(),
            LIMIT
        );
    }
}
