//! A receipt contains only static observation labels, opaque locators and counts.
//! Its disposition is exclusive even when several fields authenticate differently.
use super::*;
use crate::crypto::record_field::authenticated::{LogicalKey, Route, Slot};
use observation::{Observation, Storage};

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Totals {
    pub rows_examined: i64,
    pub absent: i64,
    pub homogeneous: i64,
    pub mixed: i64,
    pub unresolved: i64,
}

fn add(value: &mut i64, n: i64) -> AppResult<()> {
    *value = value.checked_add(n).ok_or_else(malformed)?;
    Ok(())
}

impl Totals {
    pub(super) fn include(&mut self, other: &Self) -> AppResult<()> {
        add(&mut self.rows_examined, other.rows_examined)?;
        add(&mut self.absent, other.absent)?;
        add(&mut self.homogeneous, other.homogeneous)?;
        add(&mut self.mixed, other.mixed)?;
        add(&mut self.unresolved, other.unresolved)
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Dimension {
    field: Option<String>,
    storage_version: String,
    envelope_version: String,
    binding: String,
    key_ref: Option<String>,
    route: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Bucket {
    #[serde(flatten)]
    dimension: Dimension,
    pub rows_with: i64,
    pub material_instances: i64,
    pub empty_envelopes: i64,
}

impl Bucket {
    pub(super) fn references(&self, key: &str) -> bool {
        self.material_instances > 0 && self.dimension.key_ref.as_deref() == Some(key)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Finding {
    pub field: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Receipt {
    pub row: Row,
    pub token: String,
    pub parents: Vec<String>,
    pub findings: Vec<Finding>,
    totals: Totals,
    buckets: Vec<Bucket>,
}

impl CensusRing {
    pub(super) fn attribution(&self, route: Route) -> (&'static str, Option<String>) {
        let (path, slot, provider, resource) = match &route {
            Route::Unknown => return ("unknown", None),
            Route::Direct(slot) => ("direct", slot, "local", None),
            Route::Primary(slot, logical) | Route::Fallback(slot, logical) => {
                let (provider, resource) = match logical {
                    LogicalKey::Local => ("local", None),
                    #[cfg(any(test, feature = "aws-kms"))]
                    LogicalKey::Aws(name) => ("aws-kms", Some(name.as_str())),
                    #[cfg(any(test, feature = "gcp-kms"))]
                    LogicalKey::Gcp(name) => ("gcp-kms", Some(name.as_str())),
                };
                (
                    if matches!(route, Route::Primary(..)) {
                        "primary"
                    } else {
                        "fallback"
                    },
                    slot,
                    provider,
                    resource,
                )
            }
        };
        let fallback = path == "fallback"
            || (path == "direct"
                && self
                    .proofs
                    .iter()
                    .any(|p| p.key.role == "primary" && p.key.provider != "local"));
        let role = match (fallback, slot) {
            (true, Slot::Current) => "fallback",
            (true, Slot::Previous) => "fallback_previous",
            (false, Slot::Current) => "primary",
            (false, Slot::Previous) => "previous",
        };
        (
            path,
            self.proofs
                .iter()
                .find_map(|p| p.reference(&self.deployment, role, provider, resource))
                .map(str::to_owned),
        )
    }
}

impl Receipt {
    pub(super) fn observe(state: &Checkpoint, row: &Row, observed: Observation) -> AppResult<Self> {
        let mut findings: BTreeSet<_> = observed
            .findings
            .into_iter()
            .map(|(f, reason)| (f, format!("{reason:?}")))
            .collect();
        let mut buckets = BTreeMap::<Dimension, (i64, i64)>::new();
        let mut material = BTreeSet::new();
        for (field, storage) in observed.instances {
            let mut dimension = Dimension {
                field: None,
                storage_version: String::new(),
                envelope_version: "none".into(),
                binding: "none".into(),
                key_ref: None,
                route: "none".into(),
            };
            let mut envelopes = 0;
            let mut empty_envelopes = 0;
            match storage {
                Storage::Absent => dimension.storage_version = "absent".into(),
                Storage::Sentinel => dimension.storage_version = "sentinel".into(),
                Storage::Plaintext => {
                    dimension.storage_version = "plaintext".into();
                    material.insert(dimension.clone());
                }
                Storage::Encrypted {
                    marker,
                    contract,
                    evidence,
                    empty,
                } => {
                    empty_envelopes = i64::from(empty);
                    dimension.storage_version = match marker {
                        observation::Marker::Version(n) => n.to_string(),
                        other => format!("{other:?}"),
                    };
                    dimension.envelope_version = format!("{:?}", evidence.format);
                    dimension.binding = format!("{contract:?}");
                    let (route, reference) = state.ring.attribution(evidence.route);
                    dimension.route = route.into();
                    dimension.key_ref = reference;
                    if dimension.key_ref.is_none() {
                        findings.insert((field, "UnknownKey".into()));
                    }
                    material.insert(dimension.clone());
                    envelopes = 1;
                }
            }
            for field in [None, Some(field.into())] {
                dimension.field = field;
                let (instances, empty) = buckets.entry(dimension.clone()).or_default();
                add(instances, envelopes)?;
                add(empty, empty_envelopes)?;
            }
        }
        let mut totals = Totals {
            rows_examined: 1,
            ..Default::default()
        };
        if !findings.is_empty() {
            totals.unresolved = 1;
        } else {
            match material.len() {
                0 => totals.absent = 1,
                1 => totals.homogeneous = 1,
                _ => totals.mixed = 1,
            }
        }
        Ok(Self {
            row: row.clone(),
            token: state.token(row)?,
            parents: observed
                .parents
                .iter()
                .map(|r| state.token(r))
                .collect::<AppResult<_>>()?,
            findings: findings
                .into_iter()
                .map(|(field, reason)| Finding {
                    field: field.into(),
                    reason,
                })
                .collect(),
            totals,
            buckets: buckets
                .into_iter()
                .map(
                    |(dimension, (material_instances, empty_envelopes))| Bucket {
                        empty_envelopes,
                        dimension,
                        rows_with: 1,
                        material_instances,
                    },
                )
                .collect(),
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct FamilyReport {
    pub family: String,
    pub collection: String,
    pub totals: Totals,
    pub buckets: Vec<Bucket>,
    pub head: Option<String>,
    pub pages: i64,
}

impl FamilyReport {
    pub(super) fn include(&mut self, receipt: &Receipt) -> AppResult<()> {
        self.totals.include(&receipt.totals)?;
        for bucket in &receipt.buckets {
            match self
                .buckets
                .binary_search_by(|b| b.dimension.cmp(&bucket.dimension))
            {
                Ok(n) => {
                    add(&mut self.buckets[n].rows_with, 1)?;
                    add(&mut self.buckets[n].empty_envelopes, bucket.empty_envelopes)?;
                    add(
                        &mut self.buckets[n].material_instances,
                        bucket.material_instances,
                    )?;
                }
                Err(n) => self.buckets.insert(n, bucket.clone()),
            }
        }
        Ok(())
    }
}
