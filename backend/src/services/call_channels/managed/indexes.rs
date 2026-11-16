use crate::models::managed_comms::*;
use mongodb::{
    Database, IndexModel,
    bson::{Document, doc},
    options::IndexOptions,
};
pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for (collection, keys, unique, name) in [
        (
            ManagedLinePeriod::COLLECTION_NAME,
            doc! {"rate.service_id":1,"covered_from":1},
            false,
            "managed_rental_report_interval",
        ),
        (
            ManagedVoiceAccount::COLLECTION_NAME,
            doc! {"trusted_account_id":1},
            true,
            "managed_voice_config_identity",
        ),
        (
            ManagedCharacterAllowance::COLLECTION_NAME,
            doc! {"session_id":1},
            true,
            "managed_character_call",
        ),
        (
            ManagedCharacterMonth::COLLECTION_NAME,
            doc! {"identity_key":1},
            true,
            "managed_character_month_identity",
        ),
        (
            ManagedCharacterBlock::COLLECTION_NAME,
            doc! {"session_id":1,"status":1},
            false,
            "managed_character_blocks",
        ),
        (
            CallerConfirmationBinding::COLLECTION_NAME,
            doc! {"voice_binding_id":1},
            true,
            "caller_confirmation_association",
        ),
        (
            ManagedCostPolicy::COLLECTION_NAME,
            doc! {"line_id":1},
            true,
            "managed_cost_policy_identity",
        ),
        (
            ManagedCommsCostEvidence::COLLECTION_NAME,
            doc! {"line_id":1,"created_at":1},
            false,
            "managed_cost_report",
        ),
        (
            ManagedCommsCostEvidence::COLLECTION_NAME,
            doc! {"line_id":1,"_id":1,"created_at":1},
            false,
            "managed_cost_detail_page",
        ),
        (
            ManagedCommsCostEvidence::COLLECTION_NAME,
            doc! {"tenant_owner_id":1},
            false,
            "managed_cost_erasure",
        ),
        (
            ManagedLineQuarantine::COLLECTION_NAME,
            doc! {"trusted_account_id":1,"provider_effect_id":1,"line_id":1},
            true,
            "managed_quarantine_identity",
        ),
        (
            ManagedLineRateWindow::COLLECTION_NAME,
            doc! {"key":1},
            true,
            "managed_line_rate_identity",
        ),
        (
            ManagedCallAdmission::COLLECTION_NAME,
            doc! {"admitted.line_id":1,"slot_held":1},
            false,
            "managed_line_call_slots",
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"provider":1,"number_resource_id":1},
            true,
            "managed_number_resource",
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"provider":1,"phone_number":1},
            true,
            "managed_phone_number",
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"trusted_account_id":1,"state":1},
            false,
            "managed_account_inventory",
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"state":1,"available_at":1},
            false,
            "managed_inventory_state",
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"rental_billing_service_id":1},
            false,
            "managed_rental_dependency",
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"voice.billing_service_id":1},
            false,
            "managed_voice_dependency",
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"sms.billing_service_id":1},
            false,
            "managed_sms_dependency",
        ),
        (
            crate::models::channel_adapter::ACCOUNTS,
            doc! {"catalog_service_id":1},
            false,
            "managed_canonical_dependency",
        ),
        (
            ManagedEffectRoute::COLLECTION_NAME,
            doc! {"trusted_account_id":1,"financially_finalized":1},
            false,
            "managed_account_effects",
        ),
        (
            ManagedLineClaim::COLLECTION_NAME,
            doc! {"line_id":1,"managed_claim_generation":1},
            true,
            "managed_claim_generation",
        ),
        (
            ManagedLineClaim::COLLECTION_NAME,
            doc! {"user_id":1,"state":1},
            false,
            "managed_owner_claims",
        ),
        (
            ManagedLineClaim::COLLECTION_NAME,
            doc! {"state":1,"_id":1},
            false,
            "managed_lifecycle_sweep",
        ),
        (
            ManagedLineQuote::COLLECTION_NAME,
            doc! {"kind.claim_id":1,"kind.period_index":1,"created_at":-1},
            false,
            "managed_renewal_quotes",
        ),
        (
            ManagedLineQuote::COLLECTION_NAME,
            doc! {"user_id":1,"line_id":1,"accept_before":1},
            false,
            "managed_owner_quotes",
        ),
        (
            ManagedLinePeriod::COLLECTION_NAME,
            doc! {"claim_id":1,"period_index":1},
            true,
            "managed_coverage_identity",
        ),
        (
            ManagedLinePeriod::COLLECTION_NAME,
            doc! {"billing_request_id":1},
            true,
            "managed_rental_request",
        ),
        (
            ManagedLinePeriod::COLLECTION_NAME,
            doc! {"claim_id":1,"covered_from":1,"covered_until":1},
            false,
            "managed_coverage_interval",
        ),
        (
            ManagedCommsEligibility::COLLECTION_NAME,
            doc! {"user_id":1},
            true,
            "managed_eligible_owner",
        ),
        (
            ManagedCallAdmission::COLLECTION_NAME,
            doc! {"session_id":1},
            true,
            "managed_call_admission",
        ),
        (
            ManagedCallAdmission::COLLECTION_NAME,
            doc! {"managed_claim_id":1,"financially_finalized":1},
            false,
            "managed_call_cleanup",
        ),
        (
            ManagedCallAdmission::COLLECTION_NAME,
            doc! {"hangup_deadline":1,"slot_held":1},
            false,
            "managed_call_deadline",
        ),
        (
            ManagedEffectRoute::COLLECTION_NAME,
            doc! {"trusted_account_id":1,"provider":1,"provider_effect_id":1},
            true,
            "managed_effect_identity",
        ),
        (
            ManagedEffectRoute::COLLECTION_NAME,
            doc! {"managed_claim_id":1,"financially_finalized":1},
            false,
            "managed_effect_cleanup",
        ),
        (
            ManagedInboundSms::COLLECTION_NAME,
            doc! {"trusted_account_id":1,"provider_message_id":1},
            true,
            "managed_inbound_identity",
        ),
        (
            ManagedInboundSms::COLLECTION_NAME,
            doc! {"financially_finalized":1,"created_at":1},
            false,
            "managed_inbound_recovery",
        ),
        (
            ManagedBudget::COLLECTION_NAME,
            doc! {"budget_key":1},
            true,
            "managed_budget_identity",
        ),
        (
            ManagedBudgetEntry::COLLECTION_NAME,
            doc! {"budget_id":1,"effect_key":1},
            true,
            "managed_budget_effect",
        ),
        (
            ManagedBudgetEntry::COLLECTION_NAME,
            doc! {"budget_id":1,"recognized_at":1},
            false,
            "managed_budget_capacity",
        ),
        (
            ManagedCommsCostEvidence::COLLECTION_NAME,
            doc! {"effect_id":1,"kind":1},
            true,
            "managed_cost_effect",
        ),
        (
            ManagedCommsCostEvidence::COLLECTION_NAME,
            doc! {"next_attempt_at":1},
            false,
            "managed_cost_capture",
        ),
        (
            ManagedInventoryCost::COLLECTION_NAME,
            doc! {"line_id":1,"cycle_key":1},
            true,
            "managed_inventory_cycle",
        ),
        (
            ManagedCallConfirmation::COLLECTION_NAME,
            doc! {"session_id":1,"tool_invocation_id":1,"ordinal":1},
            true,
            "managed_confirmation_identity",
        ),
        (
            ManagedCallConfirmation::COLLECTION_NAME,
            doc! {"session_id":1,"status":1},
            false,
            "managed_confirmation_slots",
        ),
        (
            ManagedLineNotice::COLLECTION_NAME,
            doc! {"key":1},
            true,
            "managed_notice_key",
        ),
        (
            ManagedLineErasure::COLLECTION_NAME,
            doc! {"user_id":1},
            true,
            "managed_erasure_owner",
        ),
        (
            ManagedLifecycleRequest::COLLECTION_NAME,
            doc! {"actor_id":1,"request_id":1},
            true,
            "managed_request_identity",
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .unique(unique)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    db.collection::<ManagedLineRateWindow>(ManagedLineRateWindow::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .name("managed_line_rate_expiry".to_owned())
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(ManagedLineQuote::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .name("managed_unaccepted_quote_expiry".to_owned())
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(ManagedLineQuote::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"kind.claim_id":1,"kind.generation":1,"kind.period_index":1})
                .options(
                    IndexOptions::builder()
                        .name("managed_automatic_renewal_quote".to_owned())
                        .unique(true)
                        .partial_filter_expression(doc! {"automatic_request_id":{"$type":"string"}})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(ManagedRentalDiagnostic::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"usage_row_id":1})
                .options(
                    IndexOptions::builder()
                        .name("managed_rental_diagnostic_row".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    // Financial and routing records deliberately have no TTL; billing retention owns them.
    Ok(())
}
