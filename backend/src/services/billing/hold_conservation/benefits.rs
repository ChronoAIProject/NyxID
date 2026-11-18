//! Eligibility for correcting benefit holds, never for charging usage.
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Row {
    #[serde(flatten)]
    hold: UsageHold,
    service_id: Option<String>,
    user_service_id: Option<String>,
    service_slug: Option<String>,
}

fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// Raw field insertion order must not change a financial observation. Array
// order remains part of the saved allocation/receipt evidence.
pub(super) fn canonical(value: Bson) -> Bson {
    match value {
        Bson::Document(doc) => Bson::Document(
            doc.into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Bson::Array(values) => Bson::Array(values.into_iter().map(canonical).collect()),
        other => other,
    }
}

pub(super) async fn valid(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    resource: &Document,
    rows: &[Document],
) -> AppResult<bool> {
    let owner = resource
        .get_str(if collection == GRANTS {
            "recipient_user_id"
        } else {
            "owner_user_id"
        })
        .ok();
    let Some(owner) = owner.filter(|s| !s.is_empty()) else {
        return Ok(false);
    };
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":owner})
        .projection(doc! {"_id":1})
        .session(&mut *session)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let rows = rows
        .iter()
        .cloned()
        .map(decode::<Row>)
        .collect::<AppResult<Vec<_>>>()?;
    if collection == GRANTS {
        let grant: CreditGrant = decode(resource.clone())?;
        let mut keys = BTreeSet::new();
        if grant.amount < Credits::ZERO
            || grant.terminal_amount < Credits::ZERO
            || grant.reserved < Credits::ZERO
            || grant.remaining < Credits::ZERO
            || grant.service_limits.iter().any(|r| {
                !key(&r.key)
                    || !keys.insert(r.key.clone())
                    || r.reserved < Credits::ZERO
                    || r.consumed < Credits::ZERO
                    || r.limit < Credits::ZERO
            })
        {
            return Ok(false);
        }
        for row in &rows {
            let Some(funding) = &row.hold.funding else {
                return Ok(false);
            };
            if row.hold.billing_owner_id != owner || funding.settled {
                return Ok(false);
            }
            let allocations: Vec<_> = funding
                .grant_reservations
                .iter()
                .filter(|r| r.grant_id == grant.id)
                .collect();
            if allocations.len() != 1 {
                return Ok(false);
            }
            let allocation = allocations[0];
            let service = row
                .service_id
                .as_deref()
                .filter(|id| Some(*id) != row.user_service_id.as_deref());
            let expected_key = super::super::service_limits::rule_key(
                &grant,
                service,
                row.service_slug.as_deref(),
            );
            if allocation.amount < Credits::ZERO
                || allocation.service_limit_key != expected_key
                || allocation
                    .service_limit_key
                    .as_ref()
                    .is_some_and(|k| !keys.contains(k))
            {
                return Ok(false);
            }
            let settle = format!("{}:grant:{}", row.hold.id, grant.id);
            let release = format!("{}:grant-release:{}", row.hold.id, grant.id);
            let extra = format!("{}:grant-extra:{}", row.hold.id, grant.id);
            let mut seen = BTreeSet::new();
            for receipt in &funding.grant_consumptions {
                if receipt.grant_id != grant.id
                    && receipt.operation_id != settle
                    && receipt.operation_id != release
                    && receipt.operation_id != extra
                {
                    continue;
                }
                if receipt.grant_id != grant.id
                    || !seen.insert(receipt.operation_id.clone())
                    || ![&settle, &release, &extra].contains(&&receipt.operation_id)
                    || receipt.service_limit_key != expected_key
                    || receipt.amount < Credits::ZERO
                    || (receipt.operation_id == settle && receipt.amount > allocation.amount)
                    || (receipt.operation_id == release && receipt.amount != Credits::ZERO)
                {
                    return Ok(false);
                }
            }
            if seen.contains(&settle) && seen.contains(&release) {
                return Ok(false);
            }
        }
    } else {
        let period: UsageAllowancePeriod = decode(resource.clone())?;
        if period.allowance_id.is_empty()
            || period.reserved_quantity < 0
            || period.consumed_quantity < 0
            || period.total_quantity < 0
        {
            return Ok(false);
        }
        for row in &rows {
            let Some(funding) = &row.hold.funding else {
                return Ok(false);
            };
            if row.hold.billing_owner_id != owner || funding.settled {
                return Ok(false);
            }
            let allocations: Vec<_> = funding
                .allowance_reservations
                .iter()
                .filter(|r| r.period_id == period.id)
                .collect();
            if allocations.len() != 1 {
                return Ok(false);
            }
            let allocation = allocations[0];
            if allocation.allowance_id != period.allowance_id || allocation.quantity < 0 {
                return Ok(false);
            }
            let settle = format!("{}:allowance:{}", row.hold.id, period.id);
            let release = format!("{}:allowance-release:{}", row.hold.id, period.id);
            let extra = format!("{}:allowance-extra:{}", row.hold.id, period.id);
            let mut seen = BTreeSet::new();
            for receipt in &funding.allowance_consumptions {
                if receipt.period_id != period.id
                    && receipt.operation_id != settle
                    && receipt.operation_id != release
                    && receipt.operation_id != extra
                {
                    continue;
                }
                if receipt.period_id != period.id
                    || receipt.allowance_id != period.allowance_id
                    || !seen.insert(receipt.operation_id.clone())
                    || ![&settle, &release, &extra].contains(&&receipt.operation_id)
                    || receipt.quantity < 0
                    || (receipt.operation_id == settle && receipt.quantity > allocation.quantity)
                    || (receipt.operation_id == release && receipt.quantity != 0)
                {
                    return Ok(false);
                }
            }
            if seen.contains(&settle) && seen.contains(&release) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) fn correction(
    collection: &str,
    resource: &Document,
    expected: &BTreeMap<String, Credits>,
    excesses: &BTreeMap<String, Credits>,
) -> AppResult<(Document, Document)> {
    let id = resource
        .get_str("_id")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut filter =
        doc! {"_id":id,"active_settlement":Bson::Null,"active_topup_expiry":Bson::Null};
    let mut set = doc! {"updated_at":bson::DateTime::now()};
    if collection == GRANTS {
        let grant: CreditGrant = decode(resource.clone())?;
        if excesses.contains_key("total") {
            filter.insert(
                "reserved",
                resource
                    .get("reserved")
                    .cloned()
                    .unwrap_or_else(|| Bson::Document(doc! {"$exists":false})),
            );
            // Set the exact expected amount, including legacy-only documents;
            // incrementing a missing exact field would create a negative hold.
            set.insert("reserved", expected["total"]);
        }
        for (index, rule) in grant.service_limits.iter().enumerate() {
            let key = format!("rule:{}", rule.key);
            if excesses.contains_key(&key) {
                filter.insert(format!("service_limits.{index}.key"), &rule.key);
                filter.insert(format!("service_limits.{index}.reserved"), rule.reserved);
                set.insert(format!("service_limits.{index}.reserved"), expected[&key]);
            }
        }
    } else {
        let period: UsageAllowancePeriod = decode(resource.clone())?;
        let value = expected["total"].pico();
        let scale = crate::models::credits::SCALE;
        if value % scale != 0 {
            return Err(AppError::Internal("Non-integral allowance hold".into()));
        }
        let value = i64::try_from(value / scale)
            .map_err(|_| AppError::Internal("Allowance hold overflow".into()))?;
        filter.insert("reserved_quantity", period.reserved_quantity);
        set.insert("reserved_quantity", value);
    }
    Ok((filter, doc! {"$set":set}))
}
