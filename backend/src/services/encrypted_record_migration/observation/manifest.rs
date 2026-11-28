//! The generation freezes this portion of its scope before reading any rows.
//! Source and identity-parent reads use whole BSON documents. Any change to
//! either document invalidates the contribution, including a change to shape.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Declaration {
    version: u32,
    entry_contract: String,
    collection: String,
    fields: Vec<String>,
    source_companions: Vec<String>,
    parent_joins: Vec<(String, String)>,
    rescan_parents: Vec<String>,
}

impl Family {
    pub(crate) fn declaration(&self) -> Declaration {
        Declaration {
            version: 1,
            entry_contract: super::super::entry_contract(self.entry),
            collection: self.collection().into(),
            fields: self.fields().map(str::to_owned).collect(),
            source_companions: self.source_companions.iter().map(|s| (*s).into()).collect(),
            parent_joins: self
                .parent_joins()
                .iter()
                .map(|&(c, p)| (c.into(), p.into()))
                .collect(),
            rescan_parents: self.rescan_parents().iter().map(|s| (*s).into()).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManifestCoverage {
    Partial,
    Complete,
}

/// Compare the entire deployed registry, including empty collections, with
/// the ordered declarations frozen for this generation and its watched scope.
pub(crate) fn coverage(frozen: &[Declaration], watched: &BTreeSet<String>) -> ManifestCoverage {
    if REGISTRY.len() != frozen.len() {
        return ManifestCoverage::Partial;
    }
    let mut deployed = families();
    for declaration in frozen {
        let Some(family) = deployed.next() else {
            return ManifestCoverage::Partial;
        };
        if family.declaration() != *declaration
            || !watched.contains(&declaration.collection)
            || !declaration
                .parent_joins
                .iter()
                .all(|(p, _)| watched.contains(p))
            || !declaration
                .rescan_parents
                .iter()
                .all(|p| watched.contains(p))
        {
            return ManifestCoverage::Partial;
        }
    }
    ManifestCoverage::Complete
}

#[cfg(test)]
mod tests;
