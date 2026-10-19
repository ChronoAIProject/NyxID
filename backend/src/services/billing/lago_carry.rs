//! Durable, per-owner/metric/rate Lago quantity carry. Rate cohorts are retained:
//! their residue can be used by later usage at the same rate and is never lost.
use super::funding::FundingSettlement;
use crate::errors::{AppError, AppResult};
use crate::models::{
    billing_lago_carry::{BillingLagoCarry, COLLECTION_NAME},
    credits::Credits,
    usage_meter::{COLLECTION_NAME as METERS, UsageMeterRow},
};
use chrono::Utc;
use mongodb::bson::{Document, doc};
use sha2::{Digest, Sha256};
pub fn cumulative_quantity(total: Credits, rate: Credits) -> AppResult<(i128, i128)> {
    if total < Credits::ZERO || rate <= Credits::ZERO {
        return Err(AppError::Internal(
            "invalid Lago carry amount or rate".into(),
        ));
    }
    // Divide first: M * 10^6 can overflow i128 even if the quotient fits.
    let whole = total.pico() / rate.pico();
    let remainder = (total.pico() % rate.pico())
        .checked_mul(1_000_000)
        .ok_or(crate::models::credits::CreditsError)?;
    let emitted = whole
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_add(remainder / rate.pico()))
        .ok_or(crate::models::credits::CreditsError)?;
    Ok((emitted, remainder % rate.pico()))
}

pub async fn commit_settlement(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    wallet: Credits,
    rate: Credits,
    mut fields: Document,
) -> AppResult<FundingSettlement> {
    if wallet == Credits::ZERO {
        // No wallet-funded quantity can affect the carry cohort. The claim
        // fenced row update is the only state change, so a majority
        // transaction and carry read would add latency without adding
        // atomicity.
        let rows = db.collection::<Document>(METERS);
        let claim = row
            .funding
            .as_ref()
            .and_then(|funding| funding.settlement_claim_id.as_deref())
            .ok_or_else(|| AppError::Internal("carry settlement lost its claim".into()))?;
        fields.insert("funding.lago_billable_quantity_micros", 0_i64);
        fields.insert("funding.lago_carry_id", None::<String>);
        let result = rows
            .update_one(
                doc! {
                    "_id": &row.id,
                    "funding.settlement_claim_id": claim,
                    "funding.settled": { "$ne": true },
                },
                doc! {
                    "$set": fields,
                    "$unset": {
                        "funding.settlement_claim_id": "",
                        "funding.settlement_claimed_at": "",
                    },
                },
            )
            .await?;
        if result.modified_count == 1 {
            return Ok(FundingSettlement {
                wallet_charge_credits: Credits::ZERO,
                lago_billable_quantity_micros: 0,
            });
        }
        if let Some(saved) = rows.find_one(doc! { "_id": &row.id }).await?
            && let Ok(funding) = saved.get_document("funding")
            && funding.get_bool("settled").unwrap_or(false)
        {
            let funding = mongodb::bson::from_document(funding.clone())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            return Ok(super::funding::settlement_from_funding(&funding));
        }
        return Err(AppError::Internal("carry settlement lost its claim".into()));
    }
    let key = hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            &row.billing_owner_id,
            &row.lago_metric_code,
            rate.to_string(),
        ))
        .map_err(|e| AppError::Internal(e.to_string()))?,
    ));
    for attempt in 0..16 {
        let key = key.clone();
        let fields = fields.clone();
        let mut session = db.client().start_session().await?;
        let db = db.clone();
        let row = row.clone();
        let result = session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .and_run2(async move |session| {
                let mut fields = fields.clone();
                let operation: AppResult<FundingSettlement> = async {
                    let rows = db.collection::<Document>(METERS);
                    let saved = rows
                        .find_one(doc! { "_id": &row.id })
                        .session(&mut *session)
                        .await?
                        .ok_or_else(|| AppError::Internal("carry usage row is missing".into()))?;
                    if let Ok(funding) = saved.get_document("funding")
                        && funding.get_bool("settled").unwrap_or(false)
                    {
                        // A worker may have lost its lease while another worker
                        // committed a different split. Return both persisted
                        // outputs together, never pair its quantity with our
                        // stale wallet share.
                        let funding = mongodb::bson::from_document(funding.clone())
                            .map_err(|e| AppError::Internal(e.to_string()))?;
                        return Ok(super::funding::settlement_from_funding(&funding));
                    }
                    let claim = row
                        .funding
                        .as_ref()
                        .and_then(|f| f.settlement_claim_id.as_deref());
                    if claim.is_none()
                        || saved
                            .get_document("funding")
                            .ok()
                            .and_then(|f| f.get_str("settlement_claim_id").ok())
                            != claim
                    {
                        return Err(AppError::Internal("carry settlement lost its claim".into()));
                    }
                    let carries = db.collection::<BillingLagoCarry>(COLLECTION_NAME);
                    let old = carries
                        .find_one(doc! { "_id": &key })
                        .session(&mut *session)
                        .await?;
                    let previous = old
                        .as_ref()
                        .map(|c| c.wallet_total)
                        .unwrap_or(Credits::ZERO);
                    let old_emitted = old
                        .as_ref()
                        .map(|c| c.emitted_quantity_micros.parse::<i128>())
                        .transpose()
                        .map_err(|_| AppError::Internal("invalid Lago carry quantity".into()))?
                        .unwrap_or(0);
                    let total = previous.checked_add(wallet)?;
                    let (emitted, remainder) = if wallet == Credits::ZERO {
                        (
                            old_emitted,
                            old.as_ref()
                                .map(|c| c.remainder_numerator.parse())
                                .transpose()
                                .map_err(|_| AppError::Internal("invalid carry remainder".into()))?
                                .unwrap_or(0),
                        )
                    } else {
                        cumulative_quantity(total, rate)?
                    };
                    let q = i64::try_from(
                        emitted
                            .checked_sub(old_emitted)
                            .ok_or(crate::models::credits::CreditsError)?,
                    )
                    .map_err(|_| AppError::Internal("Lago quantity overflow".into()))?;
                    if q < 0 {
                        return Err(AppError::Internal("Lago carry regressed".into()));
                    }
                    let carry = BillingLagoCarry {
                        id: key.clone(),
                        owner_id: row.billing_owner_id.clone(),
                        metric_code: row.lago_metric_code.clone(),
                        rate,
                        wallet_total: total,
                        emitted_quantity_micros: emitted.to_string(),
                        remainder_numerator: remainder.to_string(),
                        updated_at: Utc::now(),
                    };
                    if wallet != Credits::ZERO {
                        carries
                            .replace_one(doc! { "_id": &key }, carry)
                            .upsert(true)
                            .session(&mut *session)
                            .await?;
                    }
                    fields.insert("funding.lago_billable_quantity_micros", q);
                    fields.insert(
                        "funding.lago_carry_id",
                        if wallet == Credits::ZERO {
                            None
                        } else {
                            Some(&key)
                        },
                    );
                    let result = rows
                        .update_one(
                            doc! {
                                "_id": &row.id,
                                "funding.settlement_claim_id": claim,
                                "funding.settled": { "$ne": true },
                            },
                            doc! {
                                "$set": fields,
                                "$unset": {
                                    "funding.settlement_claim_id": "",
                                    "funding.settlement_claimed_at": "",
                                },
                            },
                        )
                        .session(&mut *session)
                        .await?;
                    if result.modified_count != 1 {
                        return Err(AppError::Internal("carry settlement lost its claim".into()));
                    }
                    Ok(FundingSettlement {
                        wallet_charge_credits: wallet,
                        lago_billable_quantity_micros: q,
                    })
                }
                .await;
                crate::services::api_key_mutation_service::transaction_result(operation)
            })
            .await;
        match result {
            Ok(quantity) => return Ok(quantity),
            Err(error) if super::ledger::is_duplicate_key_error(&error) && attempt < 15 => continue,
            Err(error) => {
                return Err(
                    crate::services::api_key_mutation_service::map_transaction_error(error),
                );
            }
        }
    }
    Err(AppError::Internal("Lago carry contention".into()))
}

#[cfg(test)]
pub async fn assign(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    wallet: Credits,
    rate: Credits,
) -> AppResult<i64> {
    let mut row = row.clone();
    let claim = format!("test:{}", row.id);
    row.funding
        .get_or_insert_with(Default::default)
        .settlement_claim_id = Some(claim.clone());
    db.collection::<Document>(METERS)
        .update_one(
            doc! { "_id": &row.id, "funding.settled": { "$ne": true } },
            doc! { "$set": { "funding.settlement_claim_id": claim } },
        )
        .await?;
    commit_settlement(
        db,
        &row,
        wallet,
        rate,
        doc! { "funding.settled": true, "funding.wallet_funded": wallet, },
    )
    .await
    .map(|settled| settled.lago_billable_quantity_micros)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_rows_floor_the_cumulative_total() {
        let rate = Credits::from_pico(3).unwrap();
        let mut total = Credits::ZERO;
        let mut last = 0;
        let mut sum = 0;
        for _ in 0..10_000 {
            total = total.checked_add(Credits::from_pico(1).unwrap()).unwrap();
            let (emitted, remainder) = cumulative_quantity(total, rate).unwrap();
            let q = emitted - last;
            sum += q;
            assert!(remainder < rate.pico());
            assert_eq!(sum, total.pico() * 1_000_000 / rate.pico());
            last = emitted;
        }
    }
}
