//! Exact covered creation identity and read-only CMA evidence validation.
use super::*;
use crate::services::assistant_cma_gateway::{CmaGatewayError, CmaResult};
use serde::Deserialize;

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash_fields(fields: &[&str]) -> String {
    let mut hash = Sha256::new();
    for field in fields {
        hash.update((field.len() as u32).to_be_bytes());
        hash.update(field.as_bytes());
    }
    hex::encode(hash.finalize())
}
pub(super) fn new_key(
    epoch: &str,
    owner: &str,
    deployment: &str,
    path: &str,
    digest: &str,
) -> AppResult<String> {
    if !lower_hex(epoch, 32) || !lower_hex(deployment, 64) || !lower_hex(digest, 64) {
        return Err(AppError::Conflict(
            "CMA creation coverage configuration is invalid".into(),
        ));
    }
    let day = Utc::now().timestamp().div_euclid(86_400).to_string();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let binding = hash_fields(&[
        "cma-creation-key-v1",
        epoch,
        &day,
        &nonce,
        owner,
        deployment,
        "POST",
        path,
        digest,
    ]);
    Ok(format!("cma1.{epoch}.{day}.{nonce}.{binding}"))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Evidence {
    Created {
        sandbox: String,
        operation: Option<String>,
    },
    Absent {
        operation: Option<String>,
    },
    Progress {
        operation: Option<String>,
    },
    Unknown,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u8,
    deployment: String,
    operation: String,
    identity_digest: String,
    result: Outcome,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    Created {
        sandbox_id: String,
        operation_id: Option<String>,
        proof_id: String,
    },
    Refused {
        code: String,
        operation_id: Option<String>,
        proof_id: String,
    },
    NeverReceived {
        proof_id: String,
    },
    InProgress {
        operation_id: Option<String>,
    },
    Unknown {
        reason: String,
    },
}
struct Identity<'a> {
    key: &'a str,
    epoch: &'a str,
    day: &'a str,
    operation: &'static str,
    digest: String,
}
fn invalid_reply() -> CmaGatewayError {
    CmaGatewayError::InvalidResponse
}
fn identity(row: &AssistantCmaCreation) -> CmaResult<Identity<'_>> {
    let key = row.covered_key.as_deref().ok_or_else(invalid_reply)?;
    let mut parts = key.split('.');
    let (Some("cma1"), Some(epoch), Some(day), Some(nonce), Some(binding), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(invalid_reply());
    };
    let operation = match (row.creates_sandbox, row.method.as_str(), row.path.as_str()) {
        (true, "POST", "/api/v1/agents") => "agent_bootstrap",
        (true, "POST", "/api/v1/sandboxes") => "sandbox",
        _ => return Err(invalid_reply()),
    };
    if !lower_hex(epoch, 32)
        || !lower_hex(nonce, 32)
        || !lower_hex(&row.deployment_fingerprint, 64)
        || !lower_hex(&row.request_hash, 64)
        || day
            .parse::<u32>()
            .ok()
            .is_none_or(|number| number.to_string() != day)
        || binding
            != hash_fields(&[
                "cma-creation-key-v1",
                epoch,
                day,
                nonce,
                &row.user_id,
                &row.deployment_fingerprint,
                "POST",
                &row.path,
                &row.request_hash,
            ])
    {
        return Err(invalid_reply());
    }
    let digest = hash_fields(&[
        "cma-creation-identity-v1",
        &row.user_id,
        &row.deployment_fingerprint,
        "POST",
        &row.path,
        key,
        "nyxid-wire-v1",
        &row.request_hash,
    ]);
    Ok(Identity {
        key,
        epoch,
        day,
        operation,
        digest,
    })
}
fn operation_matches(row: &AssistantCmaCreation, observed: Option<&str>) -> bool {
    if row.path == "/api/v1/sandboxes" {
        return observed.is_none() && row.operation_id.is_none();
    }
    observed.is_none_or(|id| lower_hex(id, 32))
        && row
            .operation_id
            .as_deref()
            .is_none_or(|saved| Some(saved) == observed)
}
fn parse(row: &AssistantCmaCreation, id: &Identity<'_>, value: Value) -> CmaResult<Evidence> {
    let response: Response = serde_json::from_value(value).map_err(|_| invalid_reply())?;
    if response.version != 1
        || response.deployment != row.deployment_fingerprint
        || response.operation != id.operation
        || response.identity_digest != id.digest
    {
        return Err(invalid_reply());
    }
    match response.result {
        Outcome::Created {
            sandbox_id,
            operation_id,
            proof_id,
        } if lower_hex(&proof_id, 64)
            && operation_matches(row, operation_id.as_deref())
            && (id.operation == "sandbox" || operation_id.is_some()) =>
        {
            validate_remote_id(&sandbox_id)?;
            if row
                .cma_sandbox_id
                .as_deref()
                .is_some_and(|saved| saved != sandbox_id)
            {
                return Err(invalid_reply());
            }
            Ok(Evidence::Created {
                sandbox: sandbox_id,
                operation: operation_id,
            })
        }
        Outcome::Refused {
            code,
            operation_id,
            proof_id,
        } if lower_hex(&proof_id, 64)
            && operation_matches(row, operation_id.as_deref())
            && matches!(
                code.as_str(),
                "sandbox_limit_reached"
                    | "trust_confirmation_required"
                    | "creation_failed"
                    | "creation_cancelled"
                    | "creation_expired"
            )
            && row.cma_agent_id.is_none()
            && row.cma_sandbox_id.is_none() =>
        {
            Ok(Evidence::Absent {
                operation: operation_id,
            })
        }
        Outcome::NeverReceived { proof_id }
            if row.operation_id.is_none()
                && row.cma_agent_id.is_none()
                && row.cma_sandbox_id.is_none()
                && proof_id == never_received_proof(id) =>
        {
            Ok(Evidence::Absent { operation: None })
        }
        Outcome::InProgress { operation_id } if operation_matches(row, operation_id.as_deref()) => {
            Ok(Evidence::Progress {
                operation: operation_id,
            })
        }
        Outcome::Unknown { reason } if !reason.is_empty() => Ok(Evidence::Unknown),
        _ => Err(invalid_reply()),
    }
}
fn never_received_proof(id: &Identity<'_>) -> String {
    hash_fields(&[
        "cma-creation-proof-v1",
        &id.digest,
        id.epoch,
        id.day,
        "never_received",
    ])
}

pub(crate) async fn lookup(
    gateway: &dyn CmaGateway,
    row: &AssistantCmaCreation,
) -> CmaResult<Evidence> {
    let id = identity(row)?;
    gateway
        .verify_creation_deployment(
            &row.user_id,
            &row.cma_service_id,
            &row.deployment_fingerprint,
        )
        .await?;
    let request = serde_json::json!({"version":1,"deployment":row.deployment_fingerprint,"method":"POST","path":row.path,"idempotency_key":id.key,
        "fingerprint":{"algorithm":"nyxid-wire-v1","digest":row.request_hash},"operation_id":row.operation_id});
    let value = gateway.creation_outcome(&row.user_id, &request).await?;
    parse(row, &id, value)
}

#[cfg(test)]
pub(super) fn fixture_response(row: &AssistantCmaCreation, result: Value) -> Value {
    let id = identity(row).unwrap();
    serde_json::json!({"version":1,"deployment":row.deployment_fingerprint,"operation":id.operation,"identity_digest":id.digest,"result":result})
}

#[cfg(test)]
pub(super) fn fixture_never_received(row: &AssistantCmaCreation) -> Value {
    let id = identity(row).unwrap();
    serde_json::json!({"kind":"never_received","proof_id":never_received_proof(&id)})
}
