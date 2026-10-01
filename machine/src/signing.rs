use std::collections::HashMap;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::Request;

const WINDOW_SECS: i64 = 60;
const MAX_NONCES: usize = 8192;

fn canonical(value: &serde_json::Value) -> Vec<u8> {
    fn append(value: &serde_json::Value, output: &mut Vec<u8>) {
        match value {
            serde_json::Value::Object(fields) => {
                let mut fields: Vec<_> = fields.iter().collect();
                fields.sort_unstable_by_key(|(key, _)| *key);
                output.push(b'{');
                for (index, (key, value)) in fields.into_iter().enumerate() {
                    if index != 0 {
                        output.push(b',');
                    }
                    serde_json::to_writer(&mut *output, key).expect("JSON key");
                    output.push(b':');
                    append(value, output);
                }
                output.push(b'}');
            }
            serde_json::Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(b',');
                    }
                    append(value, output);
                }
                output.push(b']');
            }
            other => serde_json::to_writer(output, other).expect("JSON value"),
        }
    }
    let mut output = Vec::new();
    append(value, &mut output);
    output
}

fn request_mac(request: &Request, secret: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(b"nyxid.machine.request.v1\0");
    let operation = serde_json::to_vec(&request.operation).expect("operation");
    let canonical = zeroize::Zeroizing::new(canonical(&request.parameters));
    let digest = Sha256::digest(canonical.as_slice());
    for value in [
        request.request_id.as_bytes(),
        request.node_id.as_bytes(),
        operation.as_slice(),
        digest.as_slice(),
        &request.timestamp.to_be_bytes(),
        request.nonce.as_bytes(),
    ] {
        mac.update(&(value.len() as u64).to_be_bytes());
        mac.update(value);
    }
    mac
}

pub fn sign(request: &Request, secret: &[u8]) -> String {
    hex::encode(request_mac(request, secret).finalize().into_bytes())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rejection {
    Malformed,
    Signature,
    Stale,
    Replay,
    Capacity,
}

/// Retained by the daemon across socket reconnects. Full guards fail closed.
#[derive(Default)]
pub struct ReplayGuard {
    nonces: HashMap<String, i64>,
}

impl ReplayGuard {
    pub fn verify(
        &mut self,
        request: &Request,
        node_id: &str,
        secret: &[u8],
        now: i64,
    ) -> Result<(), Rejection> {
        if request.node_id != node_id
            || uuid::Uuid::parse_str(&request.request_id).is_err()
            || uuid::Uuid::parse_str(&request.nonce).is_err()
            || secret.is_empty()
        {
            return Err(Rejection::Malformed);
        }
        if request.timestamp.abs_diff(now) > WINDOW_SECS as u64 {
            return Err(Rejection::Stale);
        }
        let provided = hex::decode(&request.signature).map_err(|_| Rejection::Signature)?;
        request_mac(request, secret)
            .verify_slice(&provided)
            .map_err(|_| Rejection::Signature)?;
        self.nonces.retain(|_, expires| *expires >= now);
        if self.nonces.contains_key(&request.nonce) {
            return Err(Rejection::Replay);
        }
        if self.nonces.len() >= MAX_NONCES {
            return Err(Rejection::Capacity);
        }
        self.nonces.insert(
            request.nonce.clone(),
            request.timestamp.saturating_add(WINDOW_SECS),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Operation;

    fn request() -> Request {
        let mut request = Request {
            request_id: uuid::Uuid::new_v4().to_string(),
            node_id: "node-a".into(),
            operation: Operation::Exec,
            parameters: serde_json::json!({"command":"true"}),
            timestamp: 1000,
            nonce: uuid::Uuid::new_v4().to_string(),
            signature: String::new(),
        };
        request.signature = sign(&request, b"test signing secret");
        request
    }

    #[test]
    fn signatures_bind_every_authority_input_and_reject_replay() {
        let request = request();
        let mut guard = ReplayGuard::default();
        assert_eq!(
            guard.verify(&request, "node-a", b"test signing secret", 1000),
            Ok(())
        );
        assert_eq!(
            guard.verify(&request, "node-a", b"test signing secret", 1000),
            Err(Rejection::Replay)
        );
        assert_eq!(
            guard.verify(&request, "node-a", b"test signing secret", 1061),
            Err(Rejection::Stale)
        );
        for index in 0..6 {
            let mut altered = request.clone();
            match index {
                0 => altered.request_id = uuid::Uuid::new_v4().to_string(),
                1 => altered.node_id = "node-b".into(),
                2 => altered.operation = Operation::ReadFile,
                3 => altered.parameters = serde_json::json!({"command":"false"}),
                4 => altered.timestamp += 1,
                _ => altered.nonce = uuid::Uuid::new_v4().to_string(),
            }
            assert!(
                ReplayGuard::default()
                    .verify(&altered, "node-a", b"test signing secret", 1000)
                    .is_err()
            );
        }
    }
}
