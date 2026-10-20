use super::credits::Credits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "billing_lago_carry";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BillingLagoCarry {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub metric_code: String,
    #[serde(with = "super::credits::whole")]
    pub rate: Credits,
    #[serde(with = "super::credits::whole")]
    pub wallet_total: Credits,
    /// Integer count of emitted millionths, encoded as a string because i128
    /// is not a BSON scalar. This is a quantity, never a monetary amount.
    pub emitted_quantity_micros: String,
    /// Remaining numerator of (wallet pico * 10^6) / rate pico. An integer
    /// string retains fractions smaller than one picocredit without rounding.
    pub remainder_numerator: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
