//! Versioned shadow completeness evidence. Each statement binds one minute and basis.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ITEMS: usize = 256;
pub const MINUTE_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Number(pub u64);

impl Serialize for Number {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Number {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let n = value.parse::<u64>().map_err(serde::de::Error::custom)?;
        if n > i64::MAX as u64 || n.to_string() != value {
            return Err(serde::de::Error::custom("invalid canonical decimal"));
        }
        Ok(Self(n))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Allocated,
    Measured,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    Cpu,
    Memory,
    Disk,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interval {
    pub start_ms: Number,
    pub end_ms: Number,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub issuer: String,
    pub subject: String,
    pub kind: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Boundary {
    Minute,
    Birth,
    EpochStart,
    Retirement,
    EpochEnd,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub basis: Basis,
    pub stream_id: String,
    pub sequence: Number,
    pub digest: String,
}

/// Every slice is derived from one immutable authoritative lifecycle capture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleSlice {
    pub interval: Interval,
    pub source_revision: Number,
    pub source_digest: String,
    pub compute: Option<Resource>,
    pub home: Option<Resource>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Lifecycle,
    Residual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Observed,
    ObservedZero,
    Partial,
    Missing,
    Estimated,
    Unresolved,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactRef {
    pub stream_id: String,
    pub fact_id: String,
    pub event_id: String,
    pub event_digest: String,
    pub evidence_revision: Number,
}

/// A measured aggregate may cover multiple intervals. Its fact reference is
/// stored once here and reused by the applicable dimension contributions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub dimension: Dimension,
    pub resource: Resource,
    pub interval: Interval,
    pub kind: ContributionKind,
    pub binding_revisions: Vec<Number>,
    pub facts: Vec<String>,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Statement {
    pub schema_version: u8,
    pub statement_id: String,
    pub revision: Number,
    pub previous_statement_digest: Option<String>,
    pub producer_namespace: String,
    pub sandbox_id: String,
    pub source_epoch: String,
    pub owner: Owner,
    pub basis: Basis,
    pub minute_start_ms: Number,
    pub scope: Interval,
    pub start_boundary: Boundary,
    pub end_boundary: Boundary,
    pub capture_revision: Number,
    pub projected_revision: Number,
    pub capture_through_ms: Number,
    pub measured_mapping_revision: Option<Number>,
    pub producer_hold: Option<String>,
    pub positions: Vec<Position>,
    pub timeline: Vec<LifecycleSlice>,
    pub facts: Vec<FactRef>,
    pub contributions: Vec<Contribution>,
}

fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
            && id.to_string() == value
    })
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_resource(resource: &Resource) -> bool {
    text(&resource.id) && text(&resource.generation)
}

fn require(condition: bool, reason: &'static str) -> Result<(), &'static str> {
    condition.then_some(()).ok_or(reason)
}

impl Statement {
    /// The caller supplies JSON parsed at its strict wire boundary. Re-serialization
    /// checks explicit nullable fields and the exact canonical scalar representation.
    pub fn from_value(value: serde_json::Value) -> Result<Self, &'static str> {
        let statement: Self =
            serde_json::from_value(value.clone()).map_err(|_| "invalid_statement")?;
        require(
            serde_json::to_value(&statement).map_err(|_| "invalid_statement")? == value,
            "noncanonical_statement_shape",
        )?;
        statement.validate()?;
        Ok(statement)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        require(
            self.schema_version == 1
                && uuid(&self.statement_id)
                && uuid(&self.source_epoch)
                && self.revision.0 > 0
                && text(&self.producer_namespace)
                && text(&self.sandbox_id)
                && text(&self.owner.issuer)
                && text(&self.owner.subject)
                && text(&self.owner.kind),
            "invalid_statement_identity",
        )?;
        require(
            (self.revision.0 == 1) == self.previous_statement_digest.is_none()
                && self.previous_statement_digest.as_deref().is_none_or(digest),
            "invalid_statement_predecessor",
        )?;
        let minute_end = self
            .minute_start_ms
            .0
            .checked_add(MINUTE_MS)
            .ok_or("invalid_minute")?;
        require(
            minute_end <= i64::MAX as u64
                && self.minute_start_ms.0.is_multiple_of(MINUTE_MS)
                && self.scope.start_ms.0 >= self.minute_start_ms.0
                && self.scope.end_ms.0 <= minute_end
                && self.scope.start_ms < self.scope.end_ms,
            "invalid_minute_scope",
        )?;
        require(
            match self.start_boundary {
                Boundary::Minute => self.scope.start_ms == self.minute_start_ms,
                Boundary::Birth | Boundary::EpochStart => true,
                _ => false,
            } && match self.end_boundary {
                Boundary::Minute => self.scope.end_ms.0 == minute_end,
                Boundary::Retirement | Boundary::EpochEnd => true,
                _ => false,
            },
            "invalid_scope_boundary",
        )?;
        require(
            self.capture_revision.0 > 0
                && self.projected_revision == self.capture_revision
                && self.capture_through_ms >= self.scope.end_ms,
            "incomplete_capture_projection",
        )?;
        require(
            self.producer_hold.as_deref().is_none_or(|reason| {
                !reason.is_empty()
                    && reason.len() <= 128
                    && reason.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            }),
            "invalid_producer_hold",
        )?;
        self.validate_positions()?;
        let expected = self.expected_contributions()?;
        self.validate_facts_and_contributions(&expected)
    }

    fn validate_positions(&self) -> Result<(), &'static str> {
        let expected = match self.basis {
            Basis::Allocated => &[Basis::Allocated][..],
            Basis::Measured => &[Basis::Allocated, Basis::Measured][..],
        };
        require(
            self.positions
                .iter()
                .map(|p| p.basis)
                .eq(expected.iter().copied())
                && self.positions.iter().all(|position| {
                    uuid(&position.stream_id) && position.sequence.0 > 0 && digest(&position.digest)
                })
                && self
                    .positions
                    .iter()
                    .map(|p| &p.stream_id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == self.positions.len()
                && (self.basis == Basis::Measured) == self.measured_mapping_revision.is_some(),
            "invalid_stream_positions",
        )
    }

    fn expected_contributions(
        &self,
    ) -> Result<BTreeSet<(Dimension, Resource, Interval)>, &'static str> {
        require(
            !self.timeline.is_empty() && self.timeline.len() <= MAX_ITEMS,
            "invalid_timeline_bound",
        )?;
        let mut cursor = self.scope.start_ms;
        let mut previous = None;
        let mut expected = BTreeSet::new();
        for slice in &self.timeline {
            require(
                slice.interval.start_ms == cursor
                    && slice.interval.end_ms > cursor
                    && slice.interval.end_ms <= self.scope.end_ms
                    && slice.source_revision.0 > 0
                    && slice.source_revision <= self.capture_revision
                    && digest(&slice.source_digest)
                    && previous.is_none_or(|revision| revision < slice.source_revision)
                    && slice.compute.as_ref().is_none_or(valid_resource)
                    && slice.home.as_ref().is_none_or(valid_resource),
                "invalid_lifecycle_timeline",
            )?;
            for (dimension, resource) in [
                (Dimension::Cpu, &slice.compute),
                (Dimension::Memory, &slice.compute),
                (Dimension::Disk, &slice.home),
            ] {
                if let Some(resource) = resource {
                    expected.insert((dimension, resource.clone(), slice.interval.clone()));
                }
            }
            previous = Some(slice.source_revision);
            cursor = slice.interval.end_ms;
        }
        require(cursor == self.scope.end_ms, "lifecycle_gap")?;
        Ok(expected)
    }

    fn validate_facts_and_contributions(
        &self,
        expected: &BTreeSet<(Dimension, Resource, Interval)>,
    ) -> Result<(), &'static str> {
        require(
            self.facts.len() <= MAX_ITEMS && self.contributions.len() <= 3 * MAX_ITEMS,
            "statement_item_bound",
        )?;
        require(
            self.facts
                .windows(2)
                .all(|items| items[0].fact_id < items[1].fact_id),
            "noncanonical_fact_set",
        )?;
        let target_stream = &self
            .positions
            .last()
            .ok_or("invalid_stream_positions")?
            .stream_id;
        let facts = self
            .facts
            .iter()
            .map(|fact| (&fact.fact_id, fact))
            .collect::<BTreeMap<_, _>>();
        require(
            self.facts.iter().all(|fact| {
                &fact.stream_id == target_stream
                    && uuid(&fact.fact_id)
                    && uuid(&fact.event_id)
                    && digest(&fact.event_digest)
                    && fact.evidence_revision.0 > 0
            }),
            "invalid_fact_reference",
        )?;
        let mut contributors = BTreeSet::new();
        let mut intervals: BTreeMap<_, Vec<&Interval>> = BTreeMap::new();
        for contribution in &self.contributions {
            let key = (
                contribution.dimension,
                contribution.resource.clone(),
                contribution.interval.clone(),
            );
            require(
                valid_resource(&contribution.resource)
                    && contribution.interval.start_ms >= self.scope.start_ms
                    && contribution.interval.start_ms < contribution.interval.end_ms
                    && contribution.interval.end_ms <= self.scope.end_ms
                    && contribution
                        .facts
                        .windows(2)
                        .all(|items| items[0] < items[1])
                    && contribution
                        .binding_revisions
                        .windows(2)
                        .all(|items| items[0] < items[1])
                    && contribution
                        .binding_revisions
                        .iter()
                        .all(|revision| revision.0 > 0),
                "invalid_contribution",
            )?;
            match contribution.kind {
                ContributionKind::Lifecycle => require(
                    expected.contains(&key) && contributors.insert(key),
                    "unexpected_or_duplicate_contribution",
                )?,
                ContributionKind::Residual => require(
                    self.basis == Basis::Measured
                        && !expected.contains(&key)
                        && !contribution.facts.is_empty(),
                    "invalid_residual_contribution",
                )?,
            }
            require(
                !matches!(
                    contribution.coverage,
                    Coverage::Observed
                        | Coverage::ObservedZero
                        | Coverage::Partial
                        | Coverage::Estimated
                ) || !contribution.facts.is_empty(),
                "coverage_without_fact",
            )?;
            for id in &contribution.facts {
                require(facts.contains_key(id), "unknown_fact_reference")?;
            }
            intervals
                .entry((contribution.dimension, &contribution.resource))
                .or_default()
                .push(&contribution.interval);
        }
        for values in intervals.values_mut() {
            values.sort();
            require(
                values
                    .windows(2)
                    .all(|items| items[0].end_ms <= items[1].start_ms),
                "overlapping_contributions",
            )?;
        }
        require(&contributors == expected, "incomplete_contributor_set")
    }
}
