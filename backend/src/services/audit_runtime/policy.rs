//! C2 admission policy. Contact freshness alarms DB-only work; only durable
//! pending age/count close it. The caller evaluates this inside its business
//! transaction before reserving capacity. Already reserved outcomes bypass new
//! admission, and narrowing must never be refused for witness reasons.
use serde::Serialize;

use crate::errors::{AppError, AppResult, AuditAdmissionReason};

pub const MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TierLimit {
    pub pending_age_secs: u64,
    pub pending_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdmissionLimits {
    pub contact_secs: u64,
    pub pending_age_secs: u64,
    pub pending_count: u64,
    pub standard: TierLimit,
    pub availability: TierLimit,
}

impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            contact_secs: 60,
            pending_age_secs: 30,
            pending_count: 10_000,
            standard: TierLimit {
                pending_age_secs: 3_600,
                pending_count: 100_000,
            },
            availability: TierLimit {
                pending_age_secs: 86_400,
                pending_count: 1_000_000,
            },
        }
    }
}

impl AdmissionLimits {
    pub fn from_env() -> Result<Self, &'static str> {
        Self::parse(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("Audit admission limits must be UTF-8"),
        })
    }

    fn parse(
        mut read: impl FnMut(&str) -> Result<Option<String>, &'static str>,
    ) -> Result<Self, &'static str> {
        let mut limits = Self::default();
        for (name, target) in [
            ("AUDIT_WITNESS_FRESH_SECS", &mut limits.contact_secs),
            (
                "AUDIT_WITNESS_PENDING_AGE_SECS",
                &mut limits.pending_age_secs,
            ),
            ("AUDIT_WITNESS_PENDING_COUNT", &mut limits.pending_count),
            (
                "AUDIT_ADMISSION_STANDARD_MAX_PENDING_AGE_SECS",
                &mut limits.standard.pending_age_secs,
            ),
            (
                "AUDIT_ADMISSION_STANDARD_MAX_PENDING_COUNT",
                &mut limits.standard.pending_count,
            ),
            (
                "AUDIT_ADMISSION_AVAILABILITY_MAX_PENDING_AGE_SECS",
                &mut limits.availability.pending_age_secs,
            ),
            (
                "AUDIT_ADMISSION_AVAILABILITY_MAX_PENDING_COUNT",
                &mut limits.availability.pending_count,
            ),
        ] {
            if let Some(value) = read(name)? {
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err("Audit admission limits must be positive decimal integers");
                }
                *target = value
                    .parse()
                    .map_err(|_| "Audit admission limit exceeds its bound")?;
            }
        }
        limits.validate()?;
        Ok(limits)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contact_secs == 0
            || self.contact_secs > MAX_AGE_SECS
            || self.pending_age_secs == 0
            || self.pending_count == 0
        {
            return Err("Audit soft limits must be positive; ages cannot exceed seven days");
        }
        for tier in [self.standard, self.availability] {
            if tier.pending_age_secs < self.pending_age_secs
                || tier.pending_age_secs > MAX_AGE_SECS
                || tier.pending_count < self.pending_count
                || tier.pending_count > i64::MAX as u64
            {
                return Err(
                    "Audit limits require positive soft <= hard and hard ages <= seven days",
                );
            }
        }
        if self.standard.pending_age_secs > self.availability.pending_age_secs
            || self.standard.pending_count > self.availability.pending_count
        {
            return Err("Audit Availability ceilings cannot be below Standard ceilings");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTier {
    Narrowing,
    Availability,
    Standard,
    Effect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessState {
    Unwitnessed,
    Ready,
    Stale,
    Backlogged,
    Diverged,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Admission {
    Open,
    Alarm,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TierAdmission {
    pub narrowing: Admission,
    pub availability: Admission,
    pub standard: Admission,
    pub effect: Admission,
}

/// Facts are obtained from the persistent head in the caller's transaction.
/// None contact age is always stale for an enrolled/configured deployment.
/// Only a newly authenticated process-local observation may supply contact age.
#[derive(Clone, Copy, Debug)]
pub struct AdmissionFacts {
    pub configured_or_enrolled: bool,
    pub state_available: bool,
    pub diverged: bool,
    pub pending_count: u64,
    pub oldest_pending_secs: u64,
    pub last_contact_secs: Option<u64>,
}

impl AdmissionFacts {
    fn soft_exceeded(self, limits: &AdmissionLimits) -> bool {
        self.last_contact_secs
            .is_none_or(|age| age >= limits.contact_secs)
            || self.oldest_pending_secs >= limits.pending_age_secs
            || self.pending_count >= limits.pending_count
    }

    fn backlogged(self, limit: TierLimit) -> bool {
        self.oldest_pending_secs >= limit.pending_age_secs
            || self.pending_count >= limit.pending_count
    }

    pub fn witness_state(self, limits: &AdmissionLimits) -> WitnessState {
        if !self.state_available {
            WitnessState::Unavailable
        } else if !self.configured_or_enrolled {
            WitnessState::Unwitnessed
        } else if self.diverged {
            WitnessState::Diverged
        } else if self.backlogged(limits.standard) {
            WitnessState::Backlogged
        } else if self.soft_exceeded(limits) {
            WitnessState::Stale
        } else {
            WitnessState::Ready
        }
    }

    pub fn admission(self, tier: AuditTier, limits: &AdmissionLimits) -> Admission {
        // A storage failure can still prevent the operation's transaction. It
        // must not become a witness-policy refusal for security narrowing.
        if tier == AuditTier::Narrowing {
            return if self.witness_state(limits) == WitnessState::Ready
                || (self.state_available && !self.configured_or_enrolled)
            {
                Admission::Open
            } else {
                Admission::Alarm
            };
        }
        if !self.state_available {
            return Admission::Closed;
        }
        if !self.configured_or_enrolled {
            return Admission::Open;
        }
        match tier {
            AuditTier::Narrowing => unreachable!(),
            AuditTier::Availability => {
                if self.backlogged(limits.availability) {
                    Admission::Closed
                } else if self.diverged || self.soft_exceeded(limits) {
                    Admission::Alarm
                } else {
                    Admission::Open
                }
            }
            AuditTier::Standard => {
                if self.diverged || self.backlogged(limits.standard) {
                    Admission::Closed
                } else if self.soft_exceeded(limits) {
                    Admission::Alarm
                } else {
                    Admission::Open
                }
            }
            AuditTier::Effect => {
                if self.diverged || self.soft_exceeded(limits) || self.backlogged(limits.standard) {
                    Admission::Closed
                } else {
                    Admission::Open
                }
            }
        }
    }

    /// An open Effect decision is only permission to seek a protected claim;
    /// it is never dispatch authorization by itself.
    pub fn require(self, tier: AuditTier, limits: &AdmissionLimits) -> AppResult<()> {
        if self.admission(tier, limits) != Admission::Closed {
            return Ok(());
        }
        if self.diverged && matches!(tier, AuditTier::Standard | AuditTier::Effect) {
            return Err(AppError::AuditWitnessDiverged);
        }
        Err(AppError::AuditAdmissionClosed {
            reason: if !self.state_available || tier == AuditTier::Effect {
                AuditAdmissionReason::WitnessUnavailable
            } else {
                AuditAdmissionReason::Backlogged
            },
        })
    }

    pub fn tiers(self, limits: &AdmissionLimits) -> TierAdmission {
        TierAdmission {
            narrowing: self.admission(AuditTier::Narrowing, limits),
            availability: self.admission(AuditTier::Availability, limits),
            standard: self.admission(AuditTier::Standard, limits),
            effect: self.admission(AuditTier::Effect, limits),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FRESH: AdmissionFacts = AdmissionFacts {
        configured_or_enrolled: true,
        state_available: true,
        diverged: false,
        pending_count: 0,
        oldest_pending_secs: 0,
        last_contact_secs: Some(0),
    };

    #[test]
    fn audit_c2_every_tier_by_state_cell() {
        use Admission::{Alarm, Closed, Open};
        let limits = AdmissionLimits::default();
        // Rows: fresh, stale, backlogged for both DB tiers, diverged.
        let rows = [
            (FRESH, [Open, Open, Open, Open]),
            (
                AdmissionFacts {
                    last_contact_secs: None,
                    ..FRESH
                },
                [Alarm, Alarm, Alarm, Closed],
            ),
            (
                AdmissionFacts {
                    oldest_pending_secs: 86_400,
                    ..FRESH
                },
                [Alarm, Closed, Closed, Closed],
            ),
            (
                AdmissionFacts {
                    diverged: true,
                    ..FRESH
                },
                [Alarm, Alarm, Closed, Closed],
            ),
        ];
        for (facts, expected) in rows {
            for (tier, expected) in [
                AuditTier::Narrowing,
                AuditTier::Availability,
                AuditTier::Standard,
                AuditTier::Effect,
            ]
            .into_iter()
            .zip(expected)
            {
                assert_eq!(
                    facts.admission(tier, &limits),
                    expected,
                    "{tier:?} {facts:?}"
                );
                assert_eq!(facts.require(tier, &limits).is_ok(), expected != Closed);
            }
        }
    }

    #[test]
    fn audit_c2_contact_never_closes_db_only_and_has_exact_soft_boundary() {
        let limits = AdmissionLimits::default();
        for contact in [None, Some(60), Some(MAX_AGE_SECS * 2)] {
            let facts = AdmissionFacts {
                last_contact_secs: contact,
                ..FRESH
            };
            assert_eq!(facts.witness_state(&limits), WitnessState::Stale);
            for tier in [
                AuditTier::Narrowing,
                AuditTier::Availability,
                AuditTier::Standard,
            ] {
                assert!(facts.require(tier, &limits).is_ok());
            }
            assert!(facts.require(AuditTier::Effect, &limits).is_err());
        }
        assert_eq!(
            AdmissionFacts {
                last_contact_secs: Some(59),
                ..FRESH
            }
            .witness_state(&limits),
            WitnessState::Ready
        );
    }

    #[test]
    fn audit_c2_each_tier_closes_at_its_own_age_or_count_ceiling() {
        let limits = AdmissionLimits::default();
        for (tier, limit) in [
            (AuditTier::Standard, limits.standard),
            (AuditTier::Availability, limits.availability),
        ] {
            for (count, age) in [
                (limit.pending_count - 1, 0),
                (0, limit.pending_age_secs - 1),
            ] {
                assert!(
                    AdmissionFacts {
                        pending_count: count,
                        oldest_pending_secs: age,
                        ..FRESH
                    }
                    .require(tier, &limits)
                    .is_ok()
                );
            }
            for (count, age) in [(limit.pending_count, 0), (0, limit.pending_age_secs)] {
                let facts = AdmissionFacts {
                    pending_count: count,
                    oldest_pending_secs: age,
                    ..FRESH
                };
                assert!(matches!(
                    facts.require(tier, &limits),
                    Err(AppError::AuditAdmissionClosed {
                        reason: AuditAdmissionReason::Backlogged
                    })
                ));
                assert!(facts.require(AuditTier::Narrowing, &limits).is_ok());
            }
        }
        let standard_only = AdmissionFacts {
            oldest_pending_secs: 3_600,
            ..FRESH
        };
        assert!(
            standard_only
                .require(AuditTier::Availability, &limits)
                .is_ok()
        );
        assert!(standard_only.require(AuditTier::Standard, &limits).is_err());
    }

    #[test]
    fn audit_c2_divergence_keeps_availability_to_ceiling_and_narrowing_always_open() {
        let limits = AdmissionLimits::default();
        for age in [0, 3_600, 86_399, 86_400, MAX_AGE_SECS * 2] {
            let facts = AdmissionFacts {
                diverged: true,
                oldest_pending_secs: age,
                ..FRESH
            };
            assert!(facts.require(AuditTier::Narrowing, &limits).is_ok());
            assert_eq!(
                facts.require(AuditTier::Availability, &limits).is_ok(),
                age < 86_400
            );
            for tier in [AuditTier::Standard, AuditTier::Effect] {
                assert!(matches!(
                    facts.require(tier, &limits),
                    Err(AppError::AuditWitnessDiverged)
                ));
            }
        }
        assert!(
            AdmissionFacts {
                state_available: false,
                ..FRESH
            }
            .require(AuditTier::Narrowing, &limits)
            .is_ok()
        );
    }

    #[test]
    fn audit_c2_unwitnessed_has_no_admission_ceiling() {
        let facts = AdmissionFacts {
            configured_or_enrolled: false,
            oldest_pending_secs: MAX_AGE_SECS * 2,
            pending_count: u64::MAX,
            last_contact_secs: None,
            ..FRESH
        };
        assert_eq!(
            facts.witness_state(&AdmissionLimits::default()),
            WitnessState::Unwitnessed
        );
        for tier in [
            AuditTier::Narrowing,
            AuditTier::Availability,
            AuditTier::Standard,
            AuditTier::Effect,
        ] {
            assert!(facts.require(tier, &AdmissionLimits::default()).is_ok());
        }
        // Unconfigured mode has no witness policy. This is not permission to
        // bypass a configured Effect intent/claim or any business control.
    }

    #[test]
    fn audit_c2_threshold_validation_rejects_disable_and_out_of_order_values() {
        for (name, value) in [
            ("AUDIT_WITNESS_FRESH_SECS", "0"),
            ("AUDIT_WITNESS_FRESH_SECS", "604801"),
            ("AUDIT_WITNESS_PENDING_COUNT", "0"),
            ("AUDIT_WITNESS_PENDING_AGE_SECS", "3601"),
            ("AUDIT_ADMISSION_STANDARD_MAX_PENDING_COUNT", "9999"),
            ("AUDIT_ADMISSION_AVAILABILITY_MAX_PENDING_COUNT", "99999"),
            ("AUDIT_ADMISSION_STANDARD_MAX_PENDING_AGE_SECS", "604801"),
            ("AUDIT_ADMISSION_AVAILABILITY_MAX_PENDING_AGE_SECS", "3599"),
            ("AUDIT_WITNESS_PENDING_COUNT", "18446744073709551616"),
            ("AUDIT_WITNESS_PENDING_COUNT", "-1"),
            ("AUDIT_WITNESS_PENDING_COUNT", "+1"),
            ("AUDIT_WITNESS_PENDING_COUNT", ""),
            ("AUDIT_WITNESS_PENDING_COUNT", " 10"),
        ] {
            assert!(
                AdmissionLimits::parse(|key| Ok((key == name).then(|| value.to_owned()))).is_err(),
                "{name}"
            );
        }
        assert_eq!(
            AdmissionLimits::parse(|_| Ok(None)).unwrap(),
            AdmissionLimits::default()
        );
        assert_eq!(
            AdmissionLimits::parse(|key| Ok((key
                == "AUDIT_ADMISSION_AVAILABILITY_MAX_PENDING_AGE_SECS")
                .then(|| "604800".into())))
            .unwrap()
            .availability
            .pending_age_secs,
            MAX_AGE_SECS
        );
    }
}
