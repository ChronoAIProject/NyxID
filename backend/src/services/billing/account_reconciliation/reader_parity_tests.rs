use super::*;
use crate::models::{
    billing_wallet::BillingWallet,
    credit_grant::{CreditGrant, CreditGrantStatus},
};
use bson::Bson;

fn wallet() -> Document {
    let now = bson::DateTime::from_millis(1_000);
    doc! {
        "_id":"wallet-id", "owner_id":"owner-id", "lago_customer_id":"customer-id",
        "plan_kind":"prepaid", "collection_state":"good",
        "balance_synced_at":now, "created_at":now, "updated_at":now,
    }
}

fn grant() -> Document {
    let now = bson::DateTime::from_millis(1_000);
    doc! {
        "_id":"grant-id", "batch_id":"batch-id", "recipient_user_id":"owner-id",
        "target_kind":"selected_users", "amount_credits":100_i64,
        "amount":Credits::from_whole(100), "remaining":Credits::from_whole(25),
        "scope":{"all_services":true}, "granted_by":"admin-id", "status":"active",
        "created_at":now, "updated_at":now, "issued_ledgered_at":now,
    }
}

fn statuses() -> Vec<CreditGrantStatus> {
    // ToSchema derives this list from the enum, rather than a second list that
    // could omit a new variant. The exhaustive match below requires its policy.
    let schema =
        serde_json::to_value(<CreditGrantStatus as utoipa::PartialSchema>::schema()).unwrap();
    let values = schema["enum"].as_array().expect("grant status enum schema");
    assert!(!values.is_empty());
    values
        .iter()
        .map(|value| serde_json::from_value(value.clone()).unwrap())
        .collect()
}

fn typed_grant_balance(row: Document) -> Option<(String, Credits)> {
    let model: CreditGrant = bson::from_document(row).unwrap();
    let needs_terminal_marker = match model.status {
        CreditGrantStatus::Active | CreditGrantStatus::Consumed => false,
        CreditGrantStatus::Expired | CreditGrantStatus::Revoked => true,
    };
    if model.issued_ledgered_at.is_none()
        || (needs_terminal_marker && model.terminal_ledgered_at.is_none())
    {
        None
    } else {
        Some((format!("grant:{}", model.id), model.remaining))
    }
}

#[test]
fn asset_balance_matches_typed_wallet_and_grant_models() {
    let decimal = |value: &str| Bson::Decimal128(value.parse().unwrap());
    let amounts = [
        None,
        Some(Bson::Int32(7)),
        Some(Bson::Int64(19)),
        Some(decimal("23.000000000001")),
        Some(decimal("-0.125")),
    ];
    for balance in &amounts {
        for pending in &amounts {
            let mut row = wallet();
            if let Some(value) = balance {
                row.insert("balance_credits", value.clone());
            }
            if let Some(value) = pending {
                row.insert("pending_lago_debits", value.clone());
            }
            let model: BillingWallet = bson::from_document(row.clone()).unwrap();
            assert_eq!(
                asset_balance(WALLETS, &row).unwrap(),
                Some((
                    format!("wallet:{}", model.owner_id),
                    model
                        .balance_credits
                        .checked_sub(model.pending_lago_debits)
                        .unwrap(),
                )),
                "wallet: {row:?}",
            );
        }
    }

    let remaining = [
        doc! {"remaining":decimal("23.000000000001")},
        doc! {"remaining":7_i32},
        doc! {"remaining":19_i64},
        doc! {"remaining_micros":7_i32},
        doc! {"remaining_micros":19_000_001_i64},
        doc! {"remaining_micros":decimal("23.000000000001")},
        doc! {"remaining":decimal("1.25"), "remaining_micros":99_000_000_i64},
        doc! {"remaining":7_i32, "remaining_micros":99_000_000_i64},
        // The compatibility macro discards even malformed legacy data when
        // the exact key is present.
        doc! {"remaining":decimal("1.25"), "remaining_micros":"ignored"},
    ];
    let markers = [
        None,
        Some(Bson::Null),
        Some(bson::DateTime::from_millis(1_000).into()),
    ];
    for status in statuses() {
        for amount in &remaining {
            for issued in &markers {
                for terminal in &markers {
                    let mut row = grant();
                    row.remove("remaining");
                    row.extend(amount.clone());
                    row.insert("status", bson::to_bson(&status).unwrap());
                    for (field, value) in [
                        ("issued_ledgered_at", issued),
                        ("terminal_ledgered_at", terminal),
                    ] {
                        row.remove(field);
                        if let Some(value) = value {
                            row.insert(field, value.clone());
                        }
                    }
                    assert_eq!(
                        asset_balance(GRANTS, &row).unwrap(),
                        typed_grant_balance(row.clone()),
                        "grant: {row:?}",
                    );
                }
            }
        }
    }
}

#[test]
fn asset_balance_rejects_malformed_shared_fields() {
    let malformed_amounts = [
        Bson::Null,
        Bson::String("25".into()),
        Bson::Double(25.0),
        Bson::Boolean(true),
        Bson::Decimal128("NaN".parse().unwrap()),
        Bson::Decimal128("0.0000000000001".parse().unwrap()),
    ];
    for value in &malformed_amounts {
        for field in ["balance_credits", "pending_lago_debits"] {
            let mut row = wallet();
            row.insert(field, value.clone());
            assert!(bson::from_document::<BillingWallet>(row.clone()).is_err());
            assert!(asset_balance(WALLETS, &row).is_err(), "{row:?}");
        }
        for field in ["remaining", "remaining_micros"] {
            let mut row = grant();
            row.remove("remaining");
            row.insert(field, value.clone());
            if field == "remaining" {
                // A valid fallback must never hide an invalid exact value.
                row.insert("remaining_micros", 25_000_000_i64);
            }
            assert!(bson::from_document::<CreditGrant>(row.clone()).is_err());
            assert!(asset_balance(GRANTS, &row).is_err(), "{row:?}");
        }
    }
    for (collection, mut row, field, value) in [
        (WALLETS, wallet(), "owner_id", Some(Bson::Int32(1))),
        (WALLETS, wallet(), "owner_id", None),
        (GRANTS, grant(), "_id", Some(Bson::Null)),
        (GRANTS, grant(), "_id", None),
        (
            GRANTS,
            grant(),
            "status",
            Some(Bson::String("unknown".into())),
        ),
        (GRANTS, grant(), "status", None),
        (GRANTS, grant(), "remaining", None),
    ] {
        row.remove(field);
        if let Some(value) = value {
            row.insert(field, value);
        }
        if collection == WALLETS {
            assert!(bson::from_document::<BillingWallet>(row.clone()).is_err());
        } else {
            assert!(bson::from_document::<CreditGrant>(row.clone()).is_err());
        }
        assert!(asset_balance(collection, &row).is_err(), "{row:?}");
    }
    for status in statuses() {
        for field in ["issued_ledgered_at", "terminal_ledgered_at"] {
            for value in [Bson::Int64(1_000), Bson::String("invalid-date".into())] {
                let mut row = grant();
                row.insert("status", bson::to_bson(&status).unwrap());
                row.insert(field, value);
                assert!(bson::from_document::<CreditGrant>(row.clone()).is_err());
                assert!(asset_balance(GRANTS, &row).is_err(), "{row:?}");
            }
        }
    }
}

#[test]
fn asset_balance_accepts_unrelated_malformed_policy_only() {
    // The bounded raw reader intentionally accepts assets whose full model
    // rejects unrelated policy (wallet plan/collection state, grant targeting/
    // scope). Those fields cannot change the checkpoint identity or amount.
    // Compare with the same document before only its policy is corrupted; the
    // shared financial fields and ledger markers must still validate above.
    for (field, value) in [
        ("plan_kind", Bson::String("invalid-plan".into())),
        ("collection_state", Bson::Null),
    ] {
        let mut row = wallet();
        row.insert("balance_credits", Credits::from_whole(25));
        row.insert("pending_lago_debits", Credits::from_whole(3));
        let model: BillingWallet = bson::from_document(row.clone()).unwrap();
        row.insert(field, value);
        assert!(bson::from_document::<BillingWallet>(row.clone()).is_err());
        assert_eq!(
            asset_balance(WALLETS, &row).unwrap(),
            Some((
                format!("wallet:{}", model.owner_id),
                model
                    .balance_credits
                    .checked_sub(model.pending_lago_debits)
                    .unwrap(),
            )),
        );
    }
    for (field, value) in [
        ("target_kind", Bson::String("invalid-target".into())),
        ("scope", Bson::String("invalid-scope".into())),
    ] {
        let mut row = grant();
        let expected = typed_grant_balance(row.clone());
        row.insert(field, value);
        assert!(bson::from_document::<CreditGrant>(row.clone()).is_err());
        assert_eq!(asset_balance(GRANTS, &row).unwrap(), expected);
    }
}
