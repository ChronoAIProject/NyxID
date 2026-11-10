use futures::{Stream, StreamExt};
use serde_json::Value;

pub(crate) fn agent_from_path(path: &str) -> Option<String> {
    // MCP builds relative forwarding paths; policy routing may retain one slash.
    let agent = path
        .strip_prefix('/')
        .unwrap_or(path)
        .strip_prefix("api/v2/agents/")?
        .strip_suffix("/responses")?;
    super::identifier(Some(agent), Some("agt_"))
}

/// CMA persists the command before emitting response.created. Dropping this
/// observation stream does not cancel it; the durable watch retrieves progress.
pub(crate) async fn read_receipt<S, B, E>(
    source: S,
    expected_agent: &str,
    limit: usize,
) -> Result<String, &'static str>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        futures::pin_mut!(source);
        let limit = limit.min(64 * 1024);
        let mut consumed = 0usize;
        let mut buffer = Vec::new();
        while let Some(chunk) = source.next().await {
            let chunk = chunk.map_err(|_| "transport")?;
            let chunk = chunk.as_ref();
            let take = chunk.len().min(limit.saturating_sub(consumed));
            buffer.extend_from_slice(&chunk[..take]);
            consumed += take;
            loop {
                let delimiter = buffer
                    .windows(2)
                    .position(|bytes| bytes == b"\n\n")
                    .map(|at| (at, 2))
                    .into_iter()
                    .chain(
                        buffer
                            .windows(4)
                            .position(|bytes| bytes == b"\r\n\r\n")
                            .map(|at| (at, 4)),
                    )
                    .min_by_key(|(at, _)| *at);
                let Some((end, length)) = delimiter else {
                    break;
                };
                let frame = std::str::from_utf8(&buffer[..end]).map_err(|_| "encoding")?;
                if let Some(receipt) = parse_frame(frame, expected_agent)? {
                    return Ok(receipt);
                }
                buffer.drain(..end + length);
            }
            if take < chunk.len() || consumed >= limit {
                return Err("size_limit");
            }
        }
        Err("missing_receipt")
    })
    .await
    .map_err(|_| "deadline")?
}

fn parse_frame(frame: &str, expected_agent: &str) -> Result<Option<String>, &'static str> {
    let mut data = Vec::new();
    let mut event = None;
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        } else if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim());
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&data.join("\n")).map_err(|_| "invalid_event")?;
    if value["type"] == "error" {
        return Err("observation_error");
    }
    if value["type"] != "response.created" {
        return if event == Some("response.created") {
            Err("invalid_event")
        } else {
            Ok(None)
        };
    }
    let response = &value["response"];
    if event.is_some_and(|event| event != "response.created")
        || super::identifier(response["id"].as_str(), Some("resp_")).is_none()
        || response["object"] != "response"
        || response["conversation"]["id"].as_str() != Some(expected_agent)
        || response["status"] != "queued"
    {
        return Err("invalid_binding");
    }
    Ok(Some(response.to_string()))
}

#[cfg(test)]
#[path = "cma_response_submission_tests.rs"]
mod tests;
