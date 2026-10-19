//! Bounds shared by capture, relay and browser input admission.
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Display {
    #[default]
    Secure,
    Dev,
}
impl Display {
    pub fn from_parameters(value: &serde_json::Value) -> Result<Self, &'static str> {
        match value
            .get("display")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("secure")
        {
            "secure" => Ok(Self::Secure),
            "dev" => Ok(Self::Dev),
            _ => Err("Unknown machine display"),
        }
    }
    pub fn key(self, node: &str) -> String {
        match self {
            Self::Secure => node.into(),
            Self::Dev => format!("{node}:dev"),
        }
    }
}

pub const MAX_WIDTH: u32 = 1920;
pub const MAX_HEIGHT: u32 = 1200;
pub const FRAME_INTERVAL: Duration = Duration::from_millis(33);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const BYTES_PER_SECOND: usize = 2 * 1024 * 1024;

pub struct FrameBudget {
    digest: Option<[u8; 32]>,
    window: Instant,
    available: usize,
}
impl Default for FrameBudget {
    fn default() -> Self {
        Self {
            digest: None,
            window: Instant::now(),
            available: BYTES_PER_SECOND,
        }
    }
}
impl FrameBudget {
    /// Hash decoded pixels, so encoder metadata cannot turn idle captures into traffic.
    pub fn changed(&self, pixels: &[u8]) -> bool {
        self.digest != Some(Sha256::digest(pixels).into())
    }
    pub fn admit(&mut self, pixels: &[u8], bytes: usize, now: Instant) -> bool {
        // Refill continuously instead of waiting for a whole second to roll
        // over. A busy scrolling page must not block the next owner input's
        // frame behind a depleted fixed window. The rate and maximum burst
        // remain bounded by BYTES_PER_SECOND.
        let elapsed = now.saturating_duration_since(self.window);
        let replenished = (elapsed.as_nanos() * BYTES_PER_SECOND as u128
            / Duration::from_secs(1).as_nanos())
        .min(BYTES_PER_SECOND as u128) as usize;
        self.available = self
            .available
            .saturating_add(replenished)
            .min(BYTES_PER_SECOND);
        self.window = now;
        if !self.changed(pixels) || bytes > crate::MAX_FRAME_BYTES || bytes > self.available {
            return false;
        }
        self.available -= bytes;
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

    #[test]
    fn a_busy_desktop_replenishes_the_next_frame_without_a_one_second_stall() {
        let mut budget = FrameBudget::default();
        let now = Instant::now();
        assert!(budget.admit(b"burst", BYTES_PER_SECOND, now));
        assert!(!budget.admit(b"input", 60_000, now));
        assert!(budget.admit(b"input", 60_000, now + FRAME_INTERVAL));
        assert!(!budget.admit(b"another", 60_000, now + FRAME_INTERVAL));
        assert!(budget.admit(b"another", 60_000, now + FRAME_INTERVAL * 2));
    }
}
