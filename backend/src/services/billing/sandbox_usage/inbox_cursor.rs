use super::inbox::InboxError;
use crate::models::sandbox_usage_inbox::Stream;
use serde::Serialize;
use sha2::{Digest, Sha256};

const PAGE_SIZE: usize = 8;
pub(super) const MAX_PENDING_DISTANCE: i64 = 32;

#[derive(Serialize)]
pub struct CursorPage {
    stream_id: String,
    contiguous_custody_sequence: String,
    custody_chain_head: String,
    contiguous_validated_sequence: String,
    validated_chain_head: String,
    highest_received_sequence: String,
    missing_ranges: Vec<MissingRange>,
    next_page: Option<String>,
}

#[derive(Serialize)]
struct MissingRange {
    first: String,
    last: String,
}

pub(super) fn snapshot(stream: Stream, page: Option<&str>) -> Result<CursorPage, InboxError> {
    let mut custody = stream.sequence;
    let mut custody_head = stream.head_digest.clone();
    let mut highest = stream.sequence;
    let mut missing = Vec::new();
    let mut hasher = Sha256::new();
    hasher.update(b"nyxid-cma-usage-cursor-v1\0");
    hasher.update(stream.namespace.as_bytes());
    hasher.update([0]);
    hasher.update(stream.stream_id.as_bytes());
    hasher.update(stream.sequence.to_be_bytes());
    hasher.update(stream.head_digest.as_bytes());
    for accepted in stream.pending {
        if accepted.sequence <= highest
            || accepted.sequence.saturating_sub(stream.sequence) > MAX_PENDING_DISTANCE
        {
            return Err(InboxError::Corrupt);
        }
        hasher.update(accepted.sequence.to_be_bytes());
        hasher.update(accepted.digest.as_bytes());
        if accepted.sequence != highest + 1 {
            missing.push(MissingRange {
                first: (highest + 1).to_string(),
                last: (accepted.sequence - 1).to_string(),
            });
        }
        if missing.is_empty() {
            custody = accepted.sequence;
            custody_head = accepted.digest;
        }
        highest = accepted.sequence;
    }
    let snapshot = hex::encode(hasher.finalize());
    let offset = match page {
        Some(page) => {
            let (version, offset) = page
                .split_once(':')
                .ok_or(InboxError::Conflict("cursor_changed"))?;
            let offset = offset
                .parse::<usize>()
                .map_err(|_| InboxError::Conflict("cursor_changed"))?;
            if version != snapshot
                || offset == 0
                || offset >= missing.len()
                || !offset.is_multiple_of(PAGE_SIZE)
            {
                return Err(InboxError::Conflict("cursor_changed"));
            }
            offset
        }
        None => 0,
    };
    let next_offset = offset + PAGE_SIZE;
    let next_page = (next_offset < missing.len()).then(|| format!("{snapshot}:{next_offset}"));
    Ok(CursorPage {
        stream_id: stream.stream_id,
        contiguous_custody_sequence: custody.to_string(),
        custody_chain_head: custody_head,
        contiguous_validated_sequence: stream.sequence.to_string(),
        validated_chain_head: stream.head_digest,
        highest_received_sequence: highest.to_string(),
        missing_ranges: missing.into_iter().skip(offset).take(PAGE_SIZE).collect(),
        next_page,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::sandbox_usage_inbox::AcceptedSequence;

    fn stream(pending: impl IntoIterator<Item = i64>) -> Stream {
        Stream {
            id: "stream-row".into(),
            namespace: "fixture".into(),
            stream_id: "stream".into(),
            owner_subject: "owner".into(),
            sandbox_id: "sandbox".into(),
            source_epoch: "epoch".into(),
            basis: "measured".into(),
            capture_start_ms: 0,
            last_fact_end_ms: 0,
            sequence: 1,
            head_id: "head-event-row".into(),
            head_digest: "a".repeat(64),
            promotion_ready: false,
            pending: pending
                .into_iter()
                .map(|sequence| AcceptedSequence {
                    sequence,
                    digest: "b".repeat(64),
                })
                .collect(),
        }
    }

    #[test]
    fn reports_durable_custody_and_validation_heads_independently() {
        let page = snapshot(stream([2, 4, 7]), None).unwrap();
        assert_eq!(page.contiguous_custody_sequence, "2");
        assert_eq!(page.custody_chain_head, "b".repeat(64));
        assert_eq!(page.contiguous_validated_sequence, "1");
        assert_eq!(page.validated_chain_head, "a".repeat(64));
        assert_eq!(page.highest_received_sequence, "7");
        assert_eq!(page.missing_ranges.len(), 2);
        assert_eq!(page.missing_ranges[0].first, "3");
        assert_eq!(page.missing_ranges[0].last, "3");
        assert_eq!(page.missing_ranges[1].first, "5");
        assert_eq!(page.missing_ranges[1].last, "6");
    }

    #[test]
    fn rejects_continuations_for_a_changed_snapshot_or_other_stream() {
        let sparse = (3..=21).step_by(2);
        let first = snapshot(stream(sparse.clone()), None).unwrap();
        let token = first.next_page.as_deref().unwrap();
        let next = snapshot(stream(sparse.clone()), Some(token)).unwrap();
        assert_eq!(first.missing_ranges.len(), 8);
        assert_eq!(next.missing_ranges.len(), 2);
        assert_eq!(next.missing_ranges[0].first, "18");
        assert_eq!(
            next.highest_received_sequence,
            first.highest_received_sequence
        );
        let changed = std::iter::once(2).chain(sparse.clone());
        assert!(matches!(
            snapshot(stream(changed), Some(token)),
            Err(InboxError::Conflict("cursor_changed"))
        ));
        let mut other = stream(sparse);
        other.namespace = "another-fixture".into();
        assert!(matches!(
            snapshot(other, Some(token)),
            Err(InboxError::Conflict("cursor_changed"))
        ));
    }
}
