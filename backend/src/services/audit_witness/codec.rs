//! Versioned, domain-separated, length-framed bytes. Never changes audit v1.
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{MAX_LINKS, PROTOCOL, WitnessError, WitnessResult, valid_hash, valid_link};
use crate::models::audit_witness::*;

fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn number(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn link(out: &mut Vec<u8>, value: &AuditLink) {
    number(out, value.key_version.into());
    number(out, value.seq);
    field(out, value.prev_hash.as_bytes());
    field(out, value.entry_hash.as_bytes());
}

pub(super) fn receipt_bytes(body: &ReceiptBody) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, b"nyxid.audit-witness.receipt");
    number(&mut out, body.protocol.into());
    field(&mut out, body.chain_id.as_bytes());
    number(&mut out, body.receipt_seq);
    field(&mut out, body.previous_receipt.as_bytes());
    number(&mut out, body.witness_time_ms);
    link(&mut out, &body.head);
    match &body.event {
        WitnessEvent::Enrollment { head } => {
            number(&mut out, 0);
            link(&mut out, head);
        }
        WitnessEvent::Advance { request } => {
            number(&mut out, 1);
            field(&mut out, request.chain_id.as_bytes());
            field(&mut out, request.request_id.as_bytes());
            field(&mut out, request.previous_receipt.as_bytes());
            number(&mut out, request.links.len() as u64);
            for value in &request.links {
                link(&mut out, value);
            }
        }
        WitnessEvent::Dispatch { request } => {
            number(&mut out, 2);
            field(&mut out, request.chain_id.as_bytes());
            field(&mut out, request.operation_id.as_bytes());
            field(&mut out, request.attempt_id.as_bytes());
            field(&mut out, request.process_id.as_bytes());
            number(&mut out, request.intent_seq);
            field(&mut out, request.intent_hash.as_bytes());
            field(&mut out, request.previous_receipt.as_bytes());
        }
    }
    out
}

pub fn receipt_digest(receipt: &SignedReceipt) -> String {
    let mut out = receipt_bytes(&receipt.body);
    field(&mut out, receipt.signature.as_bytes());
    hex::encode(Sha256::digest(out))
}

pub(super) fn observation_bytes(body: &ObservationBody) -> Vec<u8> {
    let mut out = Vec::new();
    field(&mut out, b"nyxid.audit-witness.observation");
    number(&mut out, body.protocol.into());
    field(&mut out, body.chain_id.as_bytes());
    field(&mut out, body.challenge.as_bytes());
    number(&mut out, body.witness_time_ms);
    field(&mut out, receipt_digest(&body.latest).as_bytes());
    out
}

fn signature(value: &str) -> WitnessResult<Signature> {
    if value.len() != 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(WitnessError::Corrupt);
    }
    Signature::from_slice(&hex::decode(value).map_err(|_| WitnessError::Corrupt)?)
        .map_err(|_| WitnessError::Corrupt)
}

pub fn verify_receipt(
    receipt: &SignedReceipt,
    chain: Uuid,
    key: &VerifyingKey,
) -> WitnessResult<()> {
    if receipt.body.protocol != PROTOCOL
        || receipt.body.chain_id != chain
        || !valid_link(&receipt.body.head)
        || !valid_hash(&receipt.body.previous_receipt)
        || matches!(&receipt.body.event, WitnessEvent::Advance { request } if request.links.is_empty() || request.links.len() > MAX_LINKS)
    {
        return Err(WitnessError::Corrupt);
    }
    key.verify_strict(
        &receipt_bytes(&receipt.body),
        &signature(&receipt.signature)?,
    )
    .map_err(|_| WitnessError::Corrupt)
}

pub fn verify_observation(
    value: &SignedObservation,
    chain: Uuid,
    key: &VerifyingKey,
    challenge: &str,
    earliest_ms: u64,
    latest_ms: u64,
) -> WitnessResult<()> {
    if value.body.protocol != PROTOCOL
        || value.body.chain_id != chain
        || !valid_hash(challenge)
        || value.body.challenge != challenge
        || value.body.witness_time_ms < earliest_ms
        || value.body.witness_time_ms > latest_ms
        || value.body.latest.body.witness_time_ms > value.body.witness_time_ms
    {
        return Err(WitnessError::Corrupt);
    }
    verify_receipt(&value.body.latest, chain, key)?;
    key.verify_strict(
        &observation_bytes(&value.body),
        &signature(&value.signature)?,
    )
    .map_err(|_| WitnessError::Corrupt)
}
