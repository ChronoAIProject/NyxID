use super::{
    completeness_contract::{Basis as CompletenessBasis, Coverage as CompletenessCoverage},
    completeness_inbox,
    completeness_validation::Decision,
    policy,
    shadow_buckets::Bucket,
};
use crate::{errors::AppResult, models::sandbox_usage_rates::ShadowCompletenessRef};
use mongodb::{ClientSession, Database};
use std::collections::BTreeSet;

pub(super) async fn observe_completeness(
    db: &Database,
    session: &mut ClientSession,
    bucket: &Bucket<'_>,
    pin: Option<&crate::models::sandbox_usage_rates::PricePin>,
    unresolved: &mut BTreeSet<String>,
) -> AppResult<Vec<ShadowCompletenessRef>> {
    use super::completeness_contract::Dimension;
    use crate::models::sandbox_usage_rates::Selection;
    let Some(pin) = pin else {
        return Ok(Vec::new());
    };
    let selected = [
        (Dimension::Cpu, pin.dimensions.cpu),
        (Dimension::Memory, pin.dimensions.memory),
        (Dimension::Disk, pin.dimensions.disk),
    ];
    let mut references = Vec::new();
    let mut required = 0;
    let mut validated = 0;
    for (basis, name, selection) in [
        (
            CompletenessBasis::Allocated,
            "allocated",
            Selection::Allocated,
        ),
        (CompletenessBasis::Measured, "measured", Selection::Measured),
    ] {
        if !selected.iter().any(|(_, selected)| *selected == selection) {
            continue;
        }
        required += 1;
        let statement = completeness_inbox::latest(
            db,
            session,
            bucket.namespace,
            bucket.sandbox_id,
            bucket.source_epoch,
            bucket.minute.timestamp_millis(),
            basis,
        )
        .await
        .map_err(|_| policy::unavailable("Minute completeness observation unavailable"))?;
        let Some((statement, digest, decision)) = statement else {
            continue;
        };
        let (status, reason) = match decision {
            Decision::Validated => {
                validated += 1;
                for contributor in &statement.contributions {
                    if selected.contains(&(contributor.dimension, selection))
                        && !matches!(
                            contributor.coverage,
                            CompletenessCoverage::Observed | CompletenessCoverage::ObservedZero
                        )
                    {
                        let dimension = match contributor.dimension {
                            Dimension::Cpu => "cpu",
                            Dimension::Memory => "memory",
                            Dimension::Disk => "disk",
                        };
                        unresolved.insert(format!(
                            "{name}_{dimension}:completeness_coverage_unresolved"
                        ));
                    }
                }
                ("validated", None)
            }
            Decision::Pending(reason) => ("pending", Some(reason)),
            Decision::Superseded(reason) => ("superseded", Some(reason)),
            Decision::Rejected(reason) => ("rejected", Some(reason)),
        };
        references.push(ShadowCompletenessRef {
            basis: name.into(),
            statement_id: statement.statement_id,
            statement_digest: digest,
            validation_status: status.into(),
            validation_reason: reason.map(str::to_owned),
        });
    }
    if required > 0 && required == validated {
        unresolved.remove("minute_completeness_unproven");
    }
    Ok(references)
}
