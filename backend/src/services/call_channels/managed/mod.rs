//! Managed authority is independent of catalog platform-key grants.
pub mod admission;
pub mod budgets;
pub mod calls;
pub mod confirmations;
pub mod costs;
pub mod custody;
pub mod eligibility;
pub mod indexes;
pub mod ingress;
pub mod lifecycle;
pub mod listing;
pub mod reservations;
pub mod resolver;
pub mod sms;
pub mod synthesis;
pub mod underwriting;
pub mod voice;

use crate::errors::{AppError, AppResult};
pub fn unavailable() -> AppError {
    AppError::Conflict("Managed phone line admission unavailable; review line coverage, eligibility and configuration".into())
}
pub fn conflict(message: &str) -> AppError {
    AppError::Conflict(message.into())
}
/// New admissions and configuration require an upgraded, drained roster.
/// Committed effects retain evidence, termination and recovery authority.
pub async fn require_ready(db: &mongodb::Database) -> AppResult<()> {
    crate::services::security_readiness_service::require_serving(
        db,
        crate::services::feature_flag_service::MANAGED_LINES_FLAG_KEY,
    )
    .await
}
pub async fn require_enabled(db: &mongodb::Database, owner: &str) -> AppResult<()> {
    super::binding::require_enabled(db, owner).await?;
    if db
        .collection::<crate::models::managed_comms::ManagedLineErasure>(
            crate::models::managed_comms::ManagedLineErasure::COLLECTION_NAME,
        )
        .find_one(mongodb::bson::doc! {"user_id":owner})
        .await?
        .is_some()
    {
        return Err(crate::errors::AppError::ErasurePreparing);
    }
    if !crate::services::feature_flag_service::personal_flag_enabled(
        db,
        owner,
        crate::services::feature_flag_service::MANAGED_LINES_FLAG_KEY,
    )
    .await?
    {
        return Err(unavailable());
    }
    crate::services::security_readiness_service::require_serving(
        db,
        crate::services::feature_flag_service::MANAGED_LINES_FLAG_KEY,
    )
    .await
}

#[cfg(test)]
mod tests;

/// ISO 3166-1 alpha-2, retained as a closed validation set for legal jurisdiction.
pub(super) fn valid_country(value: &str) -> bool {
    const COUNTRIES: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";
    value.len() == 2 && COUNTRIES.split_ascii_whitespace().any(|code| code == value)
}
