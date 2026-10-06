use std::collections::VecDeque;

use base64::{Engine, engine::general_purpose};
use zeroize::Zeroizing;

/// Bounded output with absolute offsets, including bytes lost to truncation.
pub struct OutputRing {
    head: Vec<u8>,
    head_capacity: usize,
    bytes: VecDeque<u8>,
    capacity: usize,
    end: u64,
}

impl OutputRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            head: Vec::new(),
            head_capacity: 0,
            bytes: VecDeque::with_capacity(capacity),
            capacity,
            end: 0,
        }
    }

    /// Reserve a small initial segment within the same total memory budget.
    pub fn with_head(capacity: usize) -> Self {
        let head_capacity = (capacity / 4).min(4096);
        Self {
            head: Vec::with_capacity(head_capacity),
            head_capacity,
            ..Self::new(capacity - head_capacity)
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.end
    }

    pub fn push(&mut self, bytes: &[u8]) {
        let initial = bytes.len().min(self.head_capacity - self.head.len());
        self.head.extend_from_slice(&bytes[..initial]);
        self.end = self.end.saturating_add(bytes.len() as u64);
        let kept = &bytes[bytes.len().saturating_sub(self.capacity)..];
        let excess = self
            .bytes
            .len()
            .saturating_add(kept.len())
            .saturating_sub(self.capacity);
        self.bytes.drain(..excess);
        self.bytes.extend(kept);
    }

    /// Foreground commands summarize both ends; job reads keep absolute-offset
    /// pagination. A later registered secret may straddle the retained head's
    /// boundary, so omit that uncertain suffix before returning the summary.
    pub fn summary_redacted(&self, limit: usize, redactor: &Redactor) -> (Vec<u8>, bool) {
        if self.end <= limit as u64 && self.end <= self.bytes.len() as u64 {
            let (_, bytes, truncated) = self.read_redacted(0, limit, redactor);
            return (bytes, truncated);
        }
        let requested_head = (limit / 2).min(self.head.len());
        let safe_head = if self.end > self.head.len() as u64 {
            self.head.len().saturating_sub(redactor.max_pattern_bytes())
        } else {
            self.head.len()
        };
        let head_end = requested_head.min(safe_head);
        let mut output = redactor.redact_window(&self.head, 0, head_end);
        if head_end < requested_head {
            output.extend_from_slice(b"[redacted]");
        }
        output.extend_from_slice(b"\n[truncated]\n");
        let tail_limit = limit.saturating_sub(requested_head);
        let (_, tail, _) = self.read_redacted(
            self.end.saturating_sub(tail_limit as u64),
            tail_limit,
            redactor,
        );
        output.extend(tail);
        (output, true)
    }

    pub fn read(&self, offset: u64, limit: usize) -> (u64, Vec<u8>, bool) {
        let start = self.end.saturating_sub(self.bytes.len() as u64);
        let offset = offset.max(start).min(self.end);
        let bytes: Vec<_> = self
            .bytes
            .iter()
            .skip((offset - start) as usize)
            .take(limit)
            .copied()
            .collect();
        (offset + bytes.len() as u64, bytes, start > 0)
    }

    /// Re-apply the live scrub registry, including patterns registered after
    /// the output was captured. Read context around both pagination boundaries.
    pub fn read_redacted(
        &self,
        offset: u64,
        limit: usize,
        redactor: &Redactor,
    ) -> (u64, Vec<u8>, bool) {
        let start = self.end.saturating_sub(self.bytes.len() as u64);
        let offset = offset.max(start).min(self.end);
        let end = offset.saturating_add(limit as u64).min(self.end);
        let padding = redactor.max_pattern_bytes() as u64;
        let context_start = offset.saturating_sub(padding).max(start);
        let context_end = end.saturating_add(padding).min(self.end);
        let bytes = Zeroizing::new(
            self.bytes
                .iter()
                .skip((context_start - start) as usize)
                .take((context_end - context_start) as usize)
                .copied()
                .collect::<Vec<_>>(),
        );
        // A discarded prefix can contain the beginning of a newly registered
        // secret. Hide the uncertain boundary rather than expose its suffix.
        let uncertain_end = if start > 0 {
            (start + padding).min(end)
        } else {
            offset
        };
        let mut output = if uncertain_end > offset {
            b"[redacted]".to_vec()
        } else {
            Vec::new()
        };
        output.extend(redactor.redact_window(
            &bytes,
            (offset.max(uncertain_end) - context_start) as usize,
            (end - context_start) as usize,
        ));
        (end, output, start > 0)
    }
}

#[derive(Default)]
pub struct Redactor {
    patterns: Vec<Zeroizing<String>>,
    bytes: usize,
}

impl Redactor {
    /// Refuse new fills when the session's bounded scrub registry is full.
    pub fn register(&mut self, secret: &str) -> Result<(), &'static str> {
        if secret.is_empty() || secret.len() > 16_384 {
            return Err("invalid secret length");
        }
        let mut additions: Vec<Zeroizing<String>> = [
            secret.to_owned(),
            general_purpose::STANDARD.encode(secret),
            general_purpose::STANDARD_NO_PAD.encode(secret),
            general_purpose::URL_SAFE.encode(secret),
            general_purpose::URL_SAFE_NO_PAD.encode(secret),
            hex::encode(secret),
            hex::encode_upper(secret),
            urlencoding::encode(secret).into_owned(),
        ]
        .into_iter()
        .map(Zeroizing::new)
        .collect();
        let encoded = Zeroizing::new(
            secret
                .bytes()
                .map(|byte| format!("%{byte:02X}"))
                .collect::<String>(),
        );
        additions.push(Zeroizing::new(encoded.to_lowercase()));
        additions.push(encoded);
        let url = urlencoding::encode(secret);
        let mut lower = Zeroizing::new(String::new());
        let mut bytes = url.as_bytes().iter().copied();
        while let Some(byte) = bytes.next() {
            lower.push(byte as char);
            if byte == b'%' {
                for byte in bytes.by_ref().take(2) {
                    lower.push((byte as char).to_ascii_lowercase());
                }
            }
        }
        additions.push(lower);
        additions.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        additions.dedup();
        additions.retain(|value| !self.patterns.iter().any(|existing| **existing == **value));
        let size: usize = additions.iter().map(|v| v.len()).sum();
        if self.bytes.saturating_add(size) > 1024 * 1024 {
            return Err("scrub registry full");
        }
        self.bytes += size;
        self.patterns.extend(additions);
        self.patterns
            .sort_by_key(|value| std::cmp::Reverse(value.len()));
        Ok(())
    }

    pub fn max_pattern_bytes(&self) -> usize {
        self.patterns.first().map_or(0, |value| value.len())
    }

    /// Redact the requested byte window using context on both sides. Callers
    /// include max_pattern_bytes() of padding so paginated reads cannot recover
    /// a secret one byte at a time. Overlapping secrets form one hidden interval.
    pub fn redact_window(&self, value: &[u8], start: usize, end: usize) -> Vec<u8> {
        let start = start.min(value.len());
        let end = end.min(value.len()).max(start);
        if self.patterns.is_empty() {
            return value[start..end].to_vec();
        }
        let mut output = Vec::new();
        let mut covered_until = 0;
        let mut emitted = false;
        for index in 0..end {
            if index >= covered_until {
                emitted = false;
            }
            for pattern in &self.patterns {
                if value[index..].starts_with(pattern.as_bytes()) {
                    covered_until = covered_until.max(index + pattern.len());
                }
            }
            if index >= start {
                if index < covered_until {
                    if !emitted {
                        output.extend_from_slice(b"[redacted]");
                        emitted = true;
                    }
                } else {
                    output.push(value[index]);
                }
            }
        }
        output
    }

    pub fn stream(&self, pending: &mut Zeroizing<Vec<u8>>, eof: bool) -> Vec<u8> {
        let mut cut = if eof {
            pending.len()
        } else {
            pending.len().saturating_sub(self.max_pattern_bytes())
        };
        let mut index = 0;
        while index < cut {
            for pattern in &self.patterns {
                if pending[index..].starts_with(pattern.as_bytes()) {
                    cut = cut.max(index + pattern.len());
                }
            }
            index += 1;
        }
        let output = self.redact_window(pending, 0, cut);
        pending.drain(..cut);
        output
    }

    pub fn redact(&self, value: &str) -> String {
        String::from_utf8_lossy(&self.redact_window(value.as_bytes(), 0, value.len())).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_bounded_and_offsets_survive_truncation() {
        let mut ring = OutputRing::new(4);
        ring.push(b"0123456789");
        assert_eq!(ring.read(0, 2), (8, b"67".to_vec(), true));
        ring.push(b"ab");
        assert_eq!(ring.read(8, 4), (12, b"89ab".to_vec(), true));
        assert_eq!(ring.read(100, 4), (12, Vec::new(), true));
    }

    #[test]
    fn foreground_preserves_both_ends_within_budget_and_hides_uncertain_secret_edges() {
        let mut ring = OutputRing::with_head(64);
        ring.push(b"BEGIN");
        ring.push(&[b'x'; 200]);
        ring.push(b"END");
        assert!(ring.head.len() + ring.bytes.len() <= 64);
        assert_eq!(ring.total_bytes(), 208);
        let (summary, truncated) = ring.summary_redacted(20, &Redactor::default());
        assert!(truncated);
        assert!(summary.starts_with(b"BEGIN"));
        assert!(summary.ends_with(b"END"));
        assert_eq!(ring.read(198, 10).0, 208);

        let secret = "a-secret-longer-than-the-retained-head";
        let mut ring = OutputRing::with_head(64);
        ring.push(secret.as_bytes());
        ring.push(&[b'x'; 100]);
        // The fill may register after a command has already emitted the value.
        let mut redactor = Redactor::default();
        redactor.register(secret).unwrap();
        let (summary, _) = ring.summary_redacted(20, &redactor);
        assert!(summary.starts_with(b"[redacted]"));
        assert!(!String::from_utf8(summary).unwrap().contains("a-secret"));
    }

    #[test]
    fn streams_redact_values_split_at_every_chunk_boundary() {
        let mut redactor = Redactor::default();
        redactor.register("secret").unwrap();
        for split in 0..20 {
            let input = b"before secret after";
            let split = split.min(input.len());
            let mut pending = Zeroizing::new(input[..split].to_vec());
            let mut output = redactor.stream(&mut pending, false);
            pending.extend_from_slice(&input[split..]);
            output.extend(redactor.stream(&mut pending, true));
            assert_eq!(output, b"before [redacted] after");
        }
    }

    #[test]
    fn pagination_and_overlapping_patterns_cannot_recover_a_login() {
        let mut redactor = Redactor::default();
        redactor.register("secret").unwrap();
        redactor.register("redact").unwrap();
        let bytes = b"prefix secret suffix";
        for start in 7..13 {
            assert_eq!(
                redactor.redact_window(bytes, start, start + 1),
                b"[redacted]"
            );
        }
        assert_eq!(redactor.redact("secret"), "[redacted]");
    }

    #[test]
    fn previously_captured_job_output_is_scrubbed_across_page_and_ring_boundaries() {
        let mut ring = OutputRing::new(16);
        ring.push(b"prefix secret suffix");
        let mut redactor = Redactor::default();
        redactor.register("secret").unwrap();
        for offset in 7..13 {
            assert_eq!(ring.read_redacted(offset, 1, &redactor).1, b"[redacted]");
        }
        let mut ring = OutputRing::new(4);
        ring.push(b"secret");
        assert_eq!(ring.read_redacted(0, 4, &redactor).1, b"[redacted]");
    }

    #[test]
    fn login_values_and_common_encodings_are_scrubbed() {
        let mut redactor = Redactor::default();
        let secret = "synthetic/password+with spaces";
        redactor.register(secret).unwrap();
        for value in [
            secret.to_owned(),
            general_purpose::STANDARD.encode(secret),
            general_purpose::URL_SAFE_NO_PAD.encode(secret),
            hex::encode(secret),
            hex::encode_upper(secret),
            urlencoding::encode(secret).into_owned(),
        ] {
            assert_eq!(redactor.redact(&value), "[redacted]");
        }
        assert_eq!(redactor.redact("ordinary output"), "ordinary output");
    }
}
