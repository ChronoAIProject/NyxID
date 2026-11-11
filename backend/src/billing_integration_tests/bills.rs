use super::*;
use crate::models::credits::Credits;
use crate::services::feature_flag_service as flags;
use crate::{
    handlers::billing::{self, BillQuery},
    test_utils::test_auth_user,
};
use axum::extract::{Path, Query, State};
fn bill() -> serde_json::Value {
    serde_json::json!({"customer_usage": {
        "currency":"USD","from_datetime":"2026-10-01T00:00:00Z","to_datetime":"2026-10-31T23:59:59Z",
        "amount_cents":1000,"taxes_amount_cents":91,"total_amount_cents":1091,
        "charges_usage":[{"billable_metric":{"code":"platform_requests","name":"Requests"},"charge":{"invoice_display_name":"AI usage"},"units":"2.5","amount_cents":1000}]
    }})
}
fn invoice(owner: &str) -> serde_json::Value {
    serde_json::json!({"lago_id":"invoice-1","number":"INV-1","issuing_date":"2026-09-30","currency":"USD","payment_status":"succeeded",
        "invoice_type":"subscription","status":"finalized","customer":{"external_id":owner},
        "sub_total_excluding_taxes_amount_cents":1000,"taxes_amount_cents":90,"sub_total_including_taxes_amount_cents":1090,
        "prepaid_credit_amount_cents":1090,"total_due_amount_cents":0,
        "applied_taxes":[{"tax_code":"gst","tax_name":"GST","tax_rate":"9","amount_cents":90}]
    })
}
#[tokio::test]
async fn bill_invoice_handlers_enforce_rollout_acl_and_receipt_ownership() {
    let db = crate::test_utils::connect_transaction_test_database("bill_handlers").await;
    let owner = insert_owner(&db).await;
    let stranger = insert_owner(&db).await;
    let mut past = invoice(&owner);
    past["invoice_type"] = serde_json::json!("credit");
    let mut draft = invoice(&owner);
    draft["status"] = serde_json::json!("draft");
    let lago = Arc::new(FakeLago {
        tax_rate: Some("9".into()),
        bill_data: Some(bill()),
        invoice_data: Some(
            serde_json::json!({"invoices":[invoice(&owner),past,draft,invoice(&stranger)],"meta":{"total_count":1}}),
        ),
        invoice: Some(crate::services::billing::lago_client::InvoiceSummary {
            lago_id: "invoice-1".into(),
            number: "INV-1".into(),
            total_amount_cents: 0,
            status: "finalized".into(),
            payment_status: "succeeded".into(),
            external_customer_id: Some(owner.clone()),
            issuing_date: Some("2026-09-30".into()),
        }),
        ..Default::default()
    });
    let state = billing_route_state(db.clone(), lago, 0);
    state.billing.ensure_wallet(&owner).await.unwrap();
    // Billing defaults on in test builds; exercise rollout denial explicitly.
    flags::set_platform_override(
        &db,
        flags::BILLING_FLAG_KEY,
        &flags::FlagTarget::Global,
        false,
        &owner,
    )
    .await
    .unwrap();
    assert!(matches!(
        billing::get_bill(
            State(state.clone()),
            test_auth_user(&owner),
            Query(BillQuery::default())
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        billing::list_invoices(
            State(state.clone()),
            test_auth_user(&owner),
            Query(BillQuery::default())
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    flags::set_platform_override(
        &db,
        flags::BILLING_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &owner,
    )
    .await
    .unwrap();
    let bill = billing::get_bill(
        State(state.clone()),
        test_auth_user(&owner),
        Query(BillQuery::default()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(bill.charges[0].name, "AI usage");
    assert_eq!(bill.charges[0].units, "2.5");
    assert_eq!(bill.total, "10.91".parse().unwrap());
    assert_eq!(bill.rounding, "0.01".parse().unwrap());
    let page = billing::list_invoices(
        State(state.clone()),
        test_auth_user(&owner),
        Query(BillQuery::default()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(page.invoices.len(), 1);
    assert_eq!(page.invoices[0].total, "10.9".parse().unwrap());
    assert_eq!(
        page.invoices[0].amount_due,
        crate::models::credits::Credits::ZERO
    );
    for id in ["invoice-1", "missing"] {
        assert!(matches!(
            billing::download_invoice(
                State(state.clone()),
                test_auth_user(&stranger),
                Path(id.into()),
                Query(BillQuery::default())
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }
    assert!(
        billing::download_invoice(
            State(state.clone()),
            test_auth_user(&owner),
            Path("invoice-1".into()),
            Query(BillQuery::default())
        )
        .await
        .is_ok()
    );
    assert!(
        billing::get_bill(
            State(state.clone()),
            test_auth_user(&stranger),
            Query(BillQuery {
                owner_id: Some(owner.clone()),
                ..Default::default()
            })
        )
        .await
        .is_err()
    );
    // Organization owners share the same resolver, requiring management access.
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection::<crate::models::org_membership::OrgMembership>(
        crate::models::org_membership::COLLECTION_NAME,
    )
    .insert_one(crate::test_utils::test_membership(
        &org,
        &owner,
        crate::models::org_membership::OrgRole::Admin,
        None,
    ))
    .await
    .unwrap();
    state.billing.ensure_wallet(&org).await.unwrap();
    assert!(
        billing::get_bill(
            State(state.clone()),
            test_auth_user(&owner),
            Query(BillQuery {
                owner_id: Some(org.clone()),
                ..Default::default()
            })
        )
        .await
        .is_ok()
    );
    assert!(
        billing::get_bill(
            State(state),
            test_auth_user(&stranger),
            Query(BillQuery {
                owner_id: Some(org),
                ..Default::default()
            })
        )
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}

#[test]
fn zero_charge_rows_are_filtered_but_nonzero_units_or_amount_remain() {
    let profile = crate::services::billing::taxes::resolve_config(
        "owner",
        "sub",
        &crate::services::billing::taxes::zero_config(),
    )
    .unwrap();
    let mut raw = bill();
    raw["customer_usage"]["charges_usage"][0]["units"] = serde_json::json!("0.000");
    raw["customer_usage"]["charges_usage"][0]["amount_cents"] = serde_json::json!(0);
    assert!(
        crate::services::billing::bill::project_current(&raw, &profile)
            .unwrap()
            .charges
            .is_empty()
    );
    raw["customer_usage"]["charges_usage"][0]["units"] = serde_json::json!("0.5");
    assert_eq!(
        crate::services::billing::bill::project_current(&raw, &profile)
            .unwrap()
            .charges
            .len(),
        1
    );
    raw["customer_usage"]["charges_usage"][0]["units"] = serde_json::json!("0");
    raw["customer_usage"]["charges_usage"][0]["amount_cents"] = serde_json::json!(1);
    assert_eq!(
        crate::services::billing::bill::project_current(&raw, &profile)
            .unwrap()
            .charges
            .len(),
        1
    );
}

#[test]
fn current_tax_breakdown_only_explains_bounded_cent_residue() {
    let mut config = crate::services::billing::taxes::zero_config();
    config.customer["taxes"] = serde_json::json!([{"code":"gst","name":"GST","rate":"9"}]);
    let profile = crate::services::billing::taxes::resolve_config("owner", "sub", &config).unwrap();
    for (tax_cents, available) in [
        (88, true),
        (92, true),
        (87, false),
        (93, false),
        (150, false),
    ] {
        let mut raw = bill();
        raw["customer_usage"]["taxes_amount_cents"] = serde_json::json!(tax_cents);
        let projected = crate::services::billing::bill::project_current(&raw, &profile).unwrap();
        assert_eq!(projected.tax_breakdown_available, available);
        assert_eq!(
            projected.tax_total,
            Credits::from_pico(i128::from(tax_cents) * (crate::models::credits::SCALE / 100))
                .unwrap()
        );
        if available {
            assert_eq!(projected.taxes.len(), 1);
        } else {
            assert!(projected.taxes.is_empty());
            assert_eq!(projected.rounding, Credits::ZERO);
        }
    }
}
