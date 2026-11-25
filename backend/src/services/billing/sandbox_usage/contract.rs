//! Inactive v1 protocol boundary. Decode raw bytes before canonicalizing them.
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

pub use super::types::*;
use super::{scalars::MAX_TIME, strict_json, validation};

pub type ContractResult<T> = Result<T, &'static str>;

#[derive(Clone, Copy)]
pub struct Clock {
    pub now_ms: u64,
    pub allowed_clock_skew_ms: u64,
}
impl Clock {
    pub(super) fn check(self, at: u64) -> ContractResult<()> {
        let ceiling = self
            .now_ms
            .checked_add(self.allowed_clock_skew_ms)
            .filter(|v| *v <= MAX_TIME)
            .ok_or("invalid_validation_clock")?;
        if at > ceiling {
            return Err("future_observation");
        }
        Ok(())
    }
}

/// Constructible only by the raw-byte decoder; accessors cannot mutate frozen evidence.
pub struct Event {
    header: Header,
    payload: Payload,
    canonical: Vec<u8>,
    digest: Digest,
}
impl Event {
    pub fn header(&self) -> &Header {
        &self.header
    }
    pub fn payload(&self) -> &Payload {
        &self.payload
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub fn digest(&self) -> &Digest {
        &self.digest
    }

    pub(super) fn custody_receipt(&self, receipt_id: String, accepted_at_ms: u64) -> Receipt {
        Receipt {
            receipt_id: Id(receipt_id),
            event_id: self.header.event_id.clone(),
            stream_id: self.header.stream_id.clone(),
            sequence: self.header.sequence,
            payload_digest: self.digest.clone(),
            accepted_at_ms: Millis(accepted_at_ms),
            custody: Custody::Persisted,
        }
    }

    /// Check the next append against the current durable stream head. The
    /// inbox must establish that the head is current in the append transaction.
    pub fn validate_successor_of(&self, head: &Event) -> ContractResult<()> {
        self.validate_stream_of(head)?;
        let before = head.header();
        let after = self.header();
        if before.sequence.get().checked_add(1) != Some(after.sequence.get())
            || after.previous_event_digest.as_ref() != Some(head.digest())
        {
            return Err("stream_chain_discontinuity");
        }
        Ok(())
    }

    pub(super) fn validate_stream_of(&self, head: &Event) -> ContractResult<()> {
        let before = head.header();
        let after = self.header();
        if before.producer_namespace != after.producer_namespace
            || before.stream_id != after.stream_id
            || before.owner_ref != after.owner_ref
            || before.sandbox_id != after.sandbox_id
            || before.source_epoch != after.source_epoch
            || before.basis != after.basis
            || before.metric_definition_version != after.metric_definition_version
            || before.algorithm_version != after.algorithm_version
        {
            return Err("stream_binding_changed");
        }
        if matches!(head.payload(), Payload::StreamSeal { .. }) {
            return Err("stream_sealed");
        }
        Ok(())
    }
}

pub fn decode_event(bytes: &[u8], clock: Clock) -> ContractResult<Event> {
    let original = strict_json::parse(bytes)?;
    let mut header = original
        .as_object()
        .cloned()
        .ok_or("event_object_required")?;
    let kind = header.remove("kind").ok_or("kind_required")?;
    let body_key = match kind.as_str() {
        Some("stream_open") => "opening",
        Some("usage") => "facts",
        Some("correction") => "correction",
        Some("stream_seal") => "seal",
        _ => return Err("unsupported_kind"),
    };
    let mut body = Map::new();
    body.insert("kind".into(), kind);
    body.insert(
        body_key.into(),
        header.remove(body_key).ok_or("body_required")?,
    );
    let header: Header = typed(Value::Object(header))?;
    let payload: Payload = typed(Value::Object(body))?;
    validation::event(&header, &payload, clock)?;
    let canonical = serde_json_canonicalizer::to_vec(&original).map_err(|_| "canonical_json")?;
    let digest = Digest(hex::encode(Sha256::digest(&canonical)));
    Ok(Event {
        header,
        payload,
        canonical,
        digest,
    })
}

pub fn decode_receipt(bytes: &[u8], clock: Clock) -> ContractResult<Receipt> {
    let receipt: Receipt = typed(strict_json::parse(bytes)?)?;
    clock.check(receipt.accepted_at_ms.get())?;
    Ok(receipt)
}

impl Receipt {
    pub fn matches(&self, event: &Event) -> bool {
        self.event_id == event.header.event_id
            && self.stream_id == event.header.stream_id
            && self.sequence == event.header.sequence
            && self.payload_digest == event.digest
            && self.accepted_at_ms >= event.header.produced_at_ms
    }
}

fn typed<T: for<'de> Deserialize<'de>>(value: Value) -> ContractResult<T> {
    serde_json::from_value(value).map_err(|_| "invalid_schema")
}
