//! Pure attempt classification and retry policy for ServicePool failover.
//!
//! This module deliberately has no database or transport side effects. The
//! handler supplies the evidence it has at the commitment boundary and the
//! policy decides whether another member may be attempted.

use std::time::Duration;

use axum::http::{Method, StatusCode, header::HeaderMap};
use chrono::{DateTime, Utc};

use crate::models::service_pool::{CooldownPolicy, FailoverPolicy, RetryTrigger};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureClass {
    NotDispatched,
    RejectedWithoutEffect,
    Ambiguous,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptEvidence {
    ConnectError,
    NodeRejectedBeforeDispatch,
    Timeout,
    NodeTimeout,
    NodeTransportAfterDispatch,
    TransportAfterDispatch,
    Upstream(StatusCode),
}

pub fn classify(evidence: AttemptEvidence) -> FailureClass {
    match evidence {
        AttemptEvidence::ConnectError | AttemptEvidence::NodeRejectedBeforeDispatch => {
            FailureClass::NotDispatched
        }
        AttemptEvidence::Upstream(status) if status == StatusCode::REQUEST_TIMEOUT => {
            FailureClass::RejectedWithoutEffect
        }
        AttemptEvidence::Upstream(status) if status == StatusCode::TOO_MANY_REQUESTS => {
            FailureClass::RejectedWithoutEffect
        }
        AttemptEvidence::Upstream(status)
            if status == StatusCode::SERVICE_UNAVAILABLE || status.as_u16() == 529 =>
        {
            // A 503 may be emitted after an upstream accepted the request;
            // treat it as ambiguous unless the provider supplies stronger
            // evidence. Unsafe methods therefore need explicit opt-in.
            FailureClass::Ambiguous
        }
        AttemptEvidence::Upstream(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => {
            FailureClass::RejectedWithoutEffect
        }
        AttemptEvidence::Timeout
        | AttemptEvidence::NodeTimeout
        | AttemptEvidence::TransportAfterDispatch
        | AttemptEvidence::NodeTransportAfterDispatch => FailureClass::Ambiguous,
        AttemptEvidence::Upstream(
            StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::GATEWAY_TIMEOUT,
        ) => FailureClass::Ambiguous,
        AttemptEvidence::Upstream(_) => FailureClass::Terminal,
    }
}

pub fn trigger_for(evidence: AttemptEvidence) -> Option<RetryTrigger> {
    match evidence {
        AttemptEvidence::ConnectError => Some(RetryTrigger::ConnectError),
        AttemptEvidence::NodeRejectedBeforeDispatch => Some(RetryTrigger::NodeOffline),
        AttemptEvidence::Timeout | AttemptEvidence::NodeTimeout => Some(RetryTrigger::Timeout),
        AttemptEvidence::TransportAfterDispatch | AttemptEvidence::NodeTransportAfterDispatch => {
            Some(RetryTrigger::TransportError)
        }
        AttemptEvidence::Upstream(status) => match status {
            StatusCode::REQUEST_TIMEOUT => Some(RetryTrigger::Http408),
            StatusCode::TOO_MANY_REQUESTS => Some(RetryTrigger::Http429),
            StatusCode::INTERNAL_SERVER_ERROR => Some(RetryTrigger::Http500),
            StatusCode::BAD_GATEWAY => Some(RetryTrigger::Http502),
            StatusCode::SERVICE_UNAVAILABLE => Some(RetryTrigger::Http503),
            StatusCode::GATEWAY_TIMEOUT => Some(RetryTrigger::Http504),
            status if status.as_u16() == 529 => Some(RetryTrigger::Http529),
            StatusCode::UNAUTHORIZED => Some(RetryTrigger::Http401),
            StatusCode::FORBIDDEN => Some(RetryTrigger::Http403),
            _ => None,
        },
    }
}

pub fn method_is_safe(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

pub fn should_retry(
    class: FailureClass,
    trigger: Option<RetryTrigger>,
    method: &Method,
    policy: &FailoverPolicy,
) -> bool {
    let Some(trigger) = trigger else { return false };
    if !policy.retry_on.contains(&trigger) {
        return false;
    }
    match class {
        FailureClass::NotDispatched | FailureClass::RejectedWithoutEffect => true,
        FailureClass::Ambiguous => policy.retry_ambiguous_dispatch || method_is_safe(method),
        FailureClass::Terminal => false,
    }
}

pub fn parse_retry_after(headers: &HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
    let value = headers.get("retry-after")?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let target = DateTime::<Utc>::from(date);
    Some(
        target
            .signed_duration_since(now)
            .to_std()
            .unwrap_or_default(),
    )
}

pub fn cooldown_for(
    policy: &CooldownPolicy,
    consecutive_failures: u32,
    retry_after: Option<Duration>,
) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(31);
    let base = u64::from(policy.base_ms).saturating_mul(1_u64 << exponent);
    let bounded = base.min(u64::from(policy.max_ms));
    if policy.honor_retry_after
        && let Some(retry_after) = retry_after
    {
        return retry_after
            .max(Duration::from_millis(bounded))
            .min(Duration::from_millis(u64::from(policy.max_ms)));
    }
    Duration::from_millis(bounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use chrono::TimeZone;

    fn policy() -> FailoverPolicy {
        FailoverPolicy::default()
    }

    #[test]
    fn classifies_statuses_without_treating_local_errors_as_upstream() {
        assert_eq!(
            classify(AttemptEvidence::Upstream(StatusCode::TOO_MANY_REQUESTS)),
            FailureClass::RejectedWithoutEffect
        );
        assert_eq!(
            classify(AttemptEvidence::Upstream(StatusCode::BAD_REQUEST)),
            FailureClass::Terminal
        );
        assert_eq!(
            classify(AttemptEvidence::Upstream(StatusCode::BAD_GATEWAY)),
            FailureClass::Ambiguous
        );
        assert_eq!(
            classify(AttemptEvidence::Upstream(StatusCode::SERVICE_UNAVAILABLE)),
            FailureClass::Ambiguous
        );
        assert_eq!(
            classify(AttemptEvidence::Upstream(StatusCode::BAD_REQUEST)),
            FailureClass::Terminal
        );
    }

    #[test]
    fn ambiguous_post_requires_opt_in_but_safe_method_does_not() {
        let mut policy = policy();
        policy.retry_on.push(RetryTrigger::Http502);
        policy.retry_on.push(RetryTrigger::Http503);
        assert!(!should_retry(
            FailureClass::Ambiguous,
            Some(RetryTrigger::Http503),
            &Method::POST,
            &policy
        ));
        assert!(!should_retry(
            FailureClass::Ambiguous,
            Some(RetryTrigger::Http502),
            &Method::POST,
            &policy
        ));
        assert!(should_retry(
            FailureClass::Ambiguous,
            Some(RetryTrigger::Http503),
            &Method::GET,
            &policy
        ));
        let mut opted = policy;
        opted.retry_ambiguous_dispatch = true;
        assert!(should_retry(
            FailureClass::Ambiguous,
            Some(RetryTrigger::Http503),
            &Method::POST,
            &opted
        ));
    }

    #[test]
    fn trigger_membership_is_independent_from_ambiguous_classification() {
        let mut policy = policy();
        policy
            .retry_on
            .retain(|trigger| *trigger != RetryTrigger::Http502);
        assert!(!policy.retry_on.contains(&RetryTrigger::Http502));
        assert!(!should_retry(
            FailureClass::Ambiguous,
            Some(RetryTrigger::Http502),
            &Method::GET,
            &policy
        ));
        assert!(should_retry(
            FailureClass::Ambiguous,
            Some(RetryTrigger::Http503),
            &Method::GET,
            &policy
        ));
    }

    #[test]
    fn dispatched_transport_failure_keeps_transport_trigger() {
        assert_eq!(
            trigger_for(AttemptEvidence::TransportAfterDispatch),
            Some(RetryTrigger::TransportError)
        );
        assert_eq!(
            trigger_for(AttemptEvidence::Timeout),
            Some(RetryTrigger::Timeout)
        );
    }

    #[test]
    fn retry_after_supports_seconds_and_http_dates_and_is_clamped() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("30"));
        assert_eq!(
            parse_retry_after(&headers, now),
            Some(Duration::from_secs(30))
        );
        headers.insert(
            "retry-after",
            HeaderValue::from_static("Thu, 01 Jan 2026 00:01:00 GMT"),
        );
        assert_eq!(
            parse_retry_after(&headers, now),
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn overload_529_is_configured_by_default_but_unsafe_replay_remains_explicit() {
        let evidence = AttemptEvidence::Upstream(StatusCode::from_u16(529).unwrap());
        assert_eq!(classify(evidence), FailureClass::Ambiguous);
        let trigger = trigger_for(evidence);
        assert_eq!(trigger, Some(RetryTrigger::Http529));
        let mut policy = policy();
        assert!(policy.retry_on.contains(&RetryTrigger::Http529));
        assert!(!should_retry(
            classify(evidence),
            trigger,
            &Method::POST,
            &policy
        ));
        assert!(should_retry(
            classify(evidence),
            trigger,
            &Method::GET,
            &policy
        ));
        policy.retry_ambiguous_dispatch = true;
        assert!(should_retry(
            classify(evidence),
            trigger,
            &Method::POST,
            &policy
        ));
    }

    #[test]
    fn retry_after_zero_cannot_erase_repeated_failure_backoff() {
        let policy = CooldownPolicy {
            base_ms: 100,
            max_ms: 350,
            ..Default::default()
        };
        for (count, expected) in [(1, 100), (2, 200), (3, 350), (99, 350)] {
            assert_eq!(
                cooldown_for(&policy, count, Some(Duration::ZERO)),
                Duration::from_millis(expected)
            );
        }
    }

    #[test]
    fn cooldown_backoff_is_bounded() {
        let policy = CooldownPolicy {
            base_ms: 100,
            max_ms: 350,
            ..CooldownPolicy::default()
        };
        assert_eq!(cooldown_for(&policy, 1, None), Duration::from_millis(100));
        assert_eq!(cooldown_for(&policy, 3, None), Duration::from_millis(350));
        assert_eq!(
            cooldown_for(&policy, 1, Some(Duration::from_secs(99))),
            Duration::from_millis(350)
        );
    }
}
