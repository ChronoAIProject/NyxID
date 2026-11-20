use super::contract::{ContractResult, Event, Payload};

impl Event {
    /// Compare against the exact validated predecessor, not a caller-supplied digest.
    /// The inbox must separately prove this predecessor is the effective stored head.
    pub fn validate_correction_of(&self, predecessor: &Event) -> ContractResult<()> {
        let Payload::Correction { correction } = self.payload() else {
            return Err("correction_required");
        };
        let before = predecessor.header();
        let after = self.header();
        if before.producer_namespace != after.producer_namespace
            || before.stream_id != after.stream_id
            || before.owner_ref != after.owner_ref
            || before.sandbox_id != after.sandbox_id
            || before.source_epoch != after.source_epoch
            || before.basis != after.basis
            || before.metric_definition_version != after.metric_definition_version
        {
            return Err("correction_stream_binding");
        }
        if correction.expected_predecessor_event_id != before.event_id
            || &correction.expected_predecessor_digest != predecessor.digest()
            || before.sequence >= after.sequence
        {
            return Err("correction_predecessor");
        }
        let old = match predecessor.payload() {
            Payload::Usage { facts } => {
                facts.iter().find(|fact| fact.fact_id == correction.fact_id)
            }
            Payload::Correction { correction: prior } => {
                Some(&prior.replacement).filter(|fact| fact.fact_id == correction.fact_id)
            }
            _ => None,
        }
        .ok_or("predecessor_fact_missing")?;
        let new = &correction.replacement;
        if old.evidence_revision != correction.expected_evidence_revision
            || old.evidence_revision.get().checked_add(1) != Some(new.evidence_revision.get())
        {
            return Err("correction_revision");
        }
        if old.fact_id != new.fact_id
            || old.bucket_start_ms != new.bucket_start_ms
            || old.start_ms != new.start_ms
            || old.end_ms != new.end_ms
            || old.resource != new.resource
            || old.quantities.keys().ne(new.quantities.keys())
        {
            return Err("correction_fact_binding");
        }
        Ok(())
    }
}
