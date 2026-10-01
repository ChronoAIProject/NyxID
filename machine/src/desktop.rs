//! Bounds shared by capture, relay and browser input admission.
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

pub const MAX_WIDTH: u32 = 1920;
pub const MAX_HEIGHT: u32 = 1200;
pub const FRAME_INTERVAL: Duration = Duration::from_millis(33);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const BYTES_PER_SECOND: usize = 2 * 1024 * 1024;

pub struct FrameBudget {
    digest: Option<[u8; 32]>,
    window: Instant,
    used: usize,
}
impl Default for FrameBudget {
    fn default() -> Self {
        Self {
            digest: None,
            window: Instant::now(),
            used: 0,
        }
    }
}
impl FrameBudget {
    /// Hash decoded pixels, so encoder metadata cannot turn idle captures into traffic.
    pub fn changed(&self, pixels: &[u8]) -> bool {
        self.digest != Some(Sha256::digest(pixels).into())
    }
    pub fn admit(&mut self, pixels: &[u8], bytes: usize, now: Instant) -> bool {
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.used = 0;
        }
        if !self.changed(pixels)
            || bytes > crate::MAX_FRAME_BYTES
            || self.used + bytes > BYTES_PER_SECOND
        {
            return false;
        }
        self.used += bytes;
        self.digest = Some(Sha256::digest(pixels).into());
        true
    }
    pub fn reset(&mut self) {
        self.digest = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_screens_have_zero_payload_and_bandwidth_is_bounded() {
        let mut budget = FrameBudget::default();
        let now = Instant::now();
        assert!(budget.admit(b"frame", 120_000, now));
        for _ in 0..1000 {
            assert!(!budget.admit(b"frame", 120_000, now));
        }
        assert!(!budget.admit(b"changed", BYTES_PER_SECOND, now));
        assert!(budget.admit(b"changed", BYTES_PER_SECOND, now + Duration::from_secs(1)));
        assert!(!budget.admit(b"more", 1, now + Duration::from_secs(1)));
        budget.reset();
        assert!(budget.changed(b"changed"));
    }
}
