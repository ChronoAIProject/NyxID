//! Positive reservation candidates, before receipts and applied locks are deducted.
//! Separate partial indexes keep zero-wallet and metering-only history out of
//! both diagnostic pages and recovery batches. Existing conservation indexes
//! remain unchanged: they also serve non-forwarded rows and malformed evidence.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Source {
    Wallet,
    Grant,
    Allowance,
}
impl Source {
    pub const ALL: [Self; 3] = [Self::Wallet, Self::Grant, Self::Allowance];
    pub fn index(self) -> &'static str {
        match self {
            Self::Wallet => "usage_meter_forwarded_wallet_hold",
            Self::Grant => "usage_meter_forwarded_grant_hold",
            Self::Allowance => "usage_meter_forwarded_allowance_hold",
        }
    }
    pub fn filter(self) -> Document {
        let mut filter = doc! {"forwarded":true,"released":false};
        let field = match self {
            Self::Wallet => {
                filter.insert("wallet_id", doc! {"$type":"string"});
                "reserved_credits"
            }
            Self::Grant => {
                filter.insert("funding.settled", doc! {"$in":[false,Bson::Null]});
                "funding.grant_reservations.amount"
            }
            Self::Allowance => {
                filter.insert("funding.settled", doc! {"$in":[false,Bson::Null]});
                "funding.allowance_reservations.quantity"
            }
        };
        // Include negative and malformed scalar values for decode diagnostics;
        // only legitimate zero/absent/null reservations are absent candidates.
        filter.insert(
            "$or",
            vec![
                doc! {field:{"$gt":Credits::ZERO}},
                doc! {field:{"$lt":Credits::ZERO}},
                doc! {field:{"$type":["string","object","bool","date","binData"]}},
            ],
        );
        if matches!(self, Self::Grant) {
            // Exact values still win when both keys exist; this is a candidate
            // superset so legacy-only reservations cannot be counted as hold-less.
            let terms = filter.get_array_mut("$or").unwrap();
            terms.push(Bson::Document(
                doc! {"funding.grant_reservations.amount_micros":{"$gt":0_i64}},
            ));
            terms.push(Bson::Document(
                doc! {"funding.grant_reservations.amount_micros":{"$lt":0_i64}},
            ));
        }
        filter
    }
    pub fn page_filter(self, after: &str) -> Document {
        let mut filter = self.filter();
        if after.is_empty() {
            return filter;
        }
        // MongoDB 8 cannot prove the partial-index implication when a cursor
        // sits beside this OR: it falls back to scanning/sorting the index.
        // Distributing the same cursor into each alternative preserves meaning
        // and lets it seek to _id on later pages as well as the first page.
        let alternatives = filter.remove("$or").unwrap().as_array().unwrap().clone();
        doc! {"$or": alternatives.into_iter().map(|term| {
            let mut branch = filter.clone();
            branch.extend(term.as_document().unwrap().clone());
            branch.insert("_id", doc! {"$gt":after});
            branch
        }).collect::<Vec<_>>()}
    }
    pub fn model(self) -> mongodb::IndexModel {
        mongodb::IndexModel::builder()
            .keys(doc! {"forwarded":1,"released":1,"_id":1})
            .options(
                mongodb::options::IndexOptions::builder()
                    .name(self.index().to_owned())
                    .partial_filter_expression(self.filter())
                    .build(),
            )
            .build()
    }
}
pub(crate) fn indexes() -> Vec<mongodb::IndexModel> {
    std::iter::once(super::inventory::index())
        .chain(Source::ALL.map(Source::model))
        .collect()
}

// The money-lane terminal transaction rechecks amounts as well as indexed selection. It
// never ends an entirely hold-less request. The terminal fence is request-wide.
pub(super) fn has_reservation(row: &Document) -> AppResult<bool> {
    let hold: UsageHold = bson::from_document(row.clone()).map_err(|_| unavailable())?;
    Ok(hold.reserved_credits > Credits::ZERO
        || hold.funding.as_ref().is_some_and(|f| {
            !f.settled
                && (f
                    .grant_reservations
                    .iter()
                    .any(|g| g.amount > Credits::ZERO)
                    || f.allowance_reservations.iter().any(|a| a.quantity > 0))
        }))
}
