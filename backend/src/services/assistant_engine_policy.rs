//! Admission, attribution and scheduling policy for NyxAgent turns.
use std::time::Duration;

/// Time from the first engine request before a human receives a pending acknowledgement.
pub const ACK_DELAY: Duration = Duration::from_secs(10);
/// Admission retry window from the first dispatch of each interactive, channel or event request.
pub const INTERACTIVE_CAPACITY_BUDGET: Duration = Duration::from_secs(120);
/// Maximum automation request admission window; original run grace bounds only its first request.
pub const AUTOMATION_CAPACITY_CAP: Duration = Duration::from_secs(600);
/// Base of the exponential full-jitter admission backoff.
pub const CAPACITY_BACKOFF_BASE: Duration = Duration::from_secs(1);
/// Maximum jitter delay, before applying a valid Retry-After minimum.
pub const CAPACITY_BACKOFF_CAP: Duration = Duration::from_secs(15);
/// Smallest supported delta-seconds Retry-After value.
pub const RETRY_AFTER_MIN: Duration = Duration::from_secs(1);
/// Largest supported delta-seconds Retry-After value.
pub const RETRY_AFTER_MAX: Duration = Duration::from_secs(60);
/// Maximum age of a captured model refusal or attributable stream fault.
pub const MODEL_ATTRIBUTION_WINDOW: Duration = Duration::from_secs(120);
/// Identical persistent non-billing failures needed to pause an automation.
pub const PERSISTENT_FAILURE_PAUSE_THRESHOLD: u32 = 3;
/// Maximum deterministic offset for an automatic recurring occurrence.
pub const DISPATCH_JITTER_MAX: Duration = Duration::from_secs(45);
/// Recurrences with less grace have no dispatch jitter.
pub const DISPATCH_JITTER_MIN_GRACE: Duration = Duration::from_secs(120);
/// At most this fraction of grace and the next interval is used by dispatch jitter.
pub const DISPATCH_JITTER_WINDOW_DIVISOR: i32 = 4;
/// Maximum time to read a dispatched engine rejection body, independent of the retry deadline.
pub const ADMISSION_ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest complete JSON rejection body that can prove a non-billable admission refusal.
pub const ADMISSION_ERROR_BODY_MAX_BYTES: usize = 64 * 1024;
/// Maximum incomplete SSE frame retained by the model fault observer.
pub const MODEL_STREAM_OBSERVER_MAX_BUFFER_BYTES: usize = 1024 * 1024;
/// Maximum hostname bytes retained in process identity metadata (DNS name bound).
pub const WORKER_HOSTNAME_MAX_BYTES: usize = 253;
/// NyxID-authored pending note; never model input or a settled assistant reply.
pub const CAPACITY_ACKNOWLEDGEMENT: &str =
    "Got it — I'm a bit busy right now and will start on this shortly.";
