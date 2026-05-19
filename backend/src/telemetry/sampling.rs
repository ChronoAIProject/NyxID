//! Deterministic per-event sampling.
//!
//! For high-volume events (e.g. `channel.message_received`), we sample a
//! small percentage deterministically — a given (event_name, distinct_id)
//! pair always produces the same sample verdict. This avoids per-user
//! jitter where 1% of one user's events lands but the next user's lands
//! 0%, and keeps longitudinal counts stable for the same user / conversation
//! across processes and deploys.
//!
//! Uses SipHash-2-4 with fixed, in-source keys. Stability across the
//! cluster and across deploys is the minimum bar; hostname- or instance-
//! derived keys would flip verdicts per replica and break longitudinal
//! analysis.
//!
//! See `docs/TELEMETRY.md` §6.5 for the sampling decision history.

// This module lands as pre-work for the Part-2 channel chunk that wires
// `should_sample_event` into `ChannelMessageReceived` / `ChannelReplySent`
// emit sites. Behavior is covered by unit tests from the moment it lands;
// production call sites follow in the same PR series.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use siphasher::sip::SipHasher24;
use std::hash::{Hash, Hasher};

/// Short stable hash of an opaque identifier, suitable for telemetry
/// property values that would otherwise be a raw UUID. Returns the first
/// 16 hex characters of SHA-256(id). Needed because `telemetry::scrub`
/// unconditionally redacts UUID-shaped strings to `[UUID_REDACTED]`; any
/// property that carries a raw UUID collapses onto that single value and
/// loses all per-entity granularity in the downstream analytics.
///
/// Use this anywhere an event schema says "node_id", "conversation_id",
/// etc. The hash is stable across processes and deploys.
pub fn hash_short_id(id: &str) -> String {
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..8])
}

/// SipHash-2-4 keys. Fixed in source — do not rotate.
///
/// Rotating these flips every sample verdict, which poisons longitudinal
/// analysis. If a future need arises to rotate (e.g., bias detected), it
/// must be a conscious one-shot migration with dashboards re-baselined.
const SIP_KEY_0: u64 = 0xc0de_cafe_f00d_beef;
const SIP_KEY_1: u64 = 0x0123_4567_89ab_cdef;

/// Returns `true` when this `(event_name, distinct_id)` should be emitted
/// given the sampling rate `sample_percent` (0..=100).
///
/// `sample_percent >= 100` → always true.
/// `sample_percent == 0` → always false.
/// Otherwise: hash the pair with SipHash-2-4 and compare `hash % 100 < sample_percent`.
///
/// Determinism is stable within a process, across processes, and across
/// deploys — as long as `SIP_KEY_0` / `SIP_KEY_1` stay fixed.
pub fn should_sample_event(event_name: &str, distinct_id: &str, sample_percent: u32) -> bool {
    if sample_percent >= 100 {
        return true;
    }
    if sample_percent == 0 {
        return false;
    }
    let mut hasher = SipHasher24::new_with_keys(SIP_KEY_0, SIP_KEY_1);
    event_name.hash(&mut hasher);
    distinct_id.hash(&mut hasher);
    let h = hasher.finish();
    (h % 100) < u64::from(sample_percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_sample_always_true() {
        assert!(should_sample_event(
            "channel.message_received",
            "user-123",
            100
        ));
        assert!(should_sample_event("x", "y", 200));
    }

    #[test]
    fn zero_sample_always_false() {
        assert!(!should_sample_event(
            "channel.message_received",
            "user-123",
            0
        ));
    }

    #[test]
    fn deterministic_same_inputs() {
        let a = should_sample_event("channel.message_received", "user-123", 10);
        let b = should_sample_event("channel.message_received", "user-123", 10);
        assert_eq!(a, b);
    }

    #[test]
    fn different_events_same_user_differ_sometimes() {
        let mut distinct = 0;
        for i in 0..100 {
            let user = format!("u{i}");
            if should_sample_event("channel.message_received", &user, 10)
                != should_sample_event("channel.reply_sent", &user, 10)
            {
                distinct += 1;
            }
        }
        // With independent hashes, expect disagreement on ~some non-trivial subset.
        assert!(distinct > 0, "event_name is not influencing the hash");
    }

    #[test]
    fn distribution_is_roughly_10_percent_at_10() {
        let mut hits = 0;
        for i in 0..1000 {
            if should_sample_event("channel.message_received", &format!("u{i}"), 10) {
                hits += 1;
            }
        }
        // Expect ~100 hits; allow 50..=170 (loose chi-square tolerance for 1000 draws).
        assert!(hits > 50 && hits < 170, "got {hits}");
    }
}
