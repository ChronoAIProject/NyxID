//! Carrier usage uses authored catalog lanes and the normal funding/ledger path.
use super::{
    binding::{self, Resolved},
    profiles,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_adapter::*,
        service_billing::{BillingMetric, PlatformUsage, PricingSyncStatus},
        usage_meter::CredentialClass,
    },
    services::billing::{BillingIngress, BillingRouteContext, MeteredProxyContext, NodeIntent},
};

pub async fn validate(state: &AppState, b: &Binding, r: &Resolved) -> AppResult<()> {
    if matches!(b.credential, CredentialBinding::ManagedLine { .. }) {
        let a = super::managed::admission::read(&state.db, &b.user_id, &b.credential).await?;
        let (cap, role, metric) = if profiles::sms(b.profile) {
            (
                a.line.sms.as_ref(),
                crate::models::managed_comms::ManagedCommsRole::Sms,
                BillingMetric::Requests,
            )
        } else {
            (
                a.line.voice.as_ref(),
                crate::models::managed_comms::ManagedCommsRole::Voice,
                BillingMetric::VoiceSeconds,
            )
        };
        state
            .billing
            .managed_price(
                &cap.ok_or_else(super::managed::unavailable)?
                    .billing_service_id,
                role,
                metric,
            )
            .await?;
        return Ok(());
    }
    // The documented inbound Answer/relay APIs cannot enforce a call-leg cap.
    // Platform liability is unavailable until Telnyx exposes a provable bound;
    // this does not affect configured SMS pricing or owner-funded BYOK calls.
    if b.profile == Profile::TelnyxRelay && r.class == CredentialClass::NyxidManagedMaster {
        return Err(AppError::VoiceProviderUnavailable);
    }

    if b.profile == Profile::ElevenlabsCustomLlm {
        if r.class == CredentialClass::NyxidManagedMaster {
            return Err(AppError::VoiceProviderUnavailable);
        }
        return Ok(());
    }
    let metric = if profiles::sms(b.profile) {
        BillingMetric::Requests
    } else {
        BillingMetric::VoiceSeconds
    };
    let lane = r.service.billing.as_ref().and_then(|p| {
        if r.class == CredentialClass::NyxidManagedMaster {
            p.platform_key_pricing.as_ref()
        } else {
            p.byok_pricing.as_ref()
        }
    });
    if r.service
        .billing
        .as_ref()
        .is_some_and(|p| p.resale_billable)
    {
        return Err(AppError::VoiceProviderUnavailable);
    }
    if let Some(lane) = lane
        && (lane.metric != metric
            || lane.sync_status != PricingSyncStatus::Synced
            || !lane.components.is_empty()
            || !lane.operations.is_empty())
    {
        return Err(AppError::VoiceProviderUnavailable);
    }
    if r.class == CredentialClass::NyxidManagedMaster {
        let CredentialBinding::Platform { catalog_service_id } = &b.credential else {
            return Err(AppError::VoiceProviderUnavailable);
        };
        let a = binding::allocation(state, b, catalog_service_id).await?;
        if lane.is_none()
            || !a.includes_all_provider_charges
            || a.tariff_version.is_empty()
            || !a.tariff_source.starts_with("https://")
            || chrono::Utc::now()
                .signed_duration_since(a.tariff_verified_at)
                .num_days()
                > 30
            || !state.billing.billing_enabled()
            || !state.billing.lago_configured()
        {
            return Err(AppError::VoiceProviderUnavailable);
        }
    }
    Ok(())
}
pub async fn context(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    id: &str,
) -> AppResult<BillingRouteContext> {
    validate(state, b, r).await?;
    let managed_service;
    let r = if matches!(b.credential, CredentialBinding::ManagedLine { .. }) {
        let a = super::managed::admission::read(&state.db, &b.user_id, &b.credential).await?;
        let cap = if profiles::sms(b.profile) {
            a.line.sms.as_ref()
        } else {
            a.line.voice.as_ref()
        }
        .ok_or_else(super::managed::unavailable)?;
        let service = state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .find_one(mongodb::bson::doc! {"_id":&cap.billing_service_id})
            .await?
            .ok_or_else(super::managed::unavailable)?;
        managed_service = Resolved {
            service,
            key: zeroize::Zeroizing::new(String::new()),
            identity: r.identity.clone(),
            class: r.class,
            resource_owner: b.user_id.clone(),
            connection_id: None,
        };
        &managed_service
    } else {
        r
    };
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_ordinary_execution(&b.user_id, &r.resource_owner, r.class)
        .await?;
    let pricing = r.service.billing.as_ref().filter(|p| {
        if r.class == CredentialClass::NyxidManagedMaster {
            p.platform_key_pricing.is_some()
        } else {
            p.byok_pricing.is_some()
        }
    });
    let mut ctx = BillingRouteContext::new(
        BillingIngress::ChannelOutbound,
        id.into(),
        owner.owner_id,
        b.user_id.clone(),
        None,
        r.connection_id.clone(),
        Some(r.service.id.clone()),
        Some(r.service.slug.clone()),
        NodeIntent::Direct,
        if profiles::provider(b.profile) == "twilio" {
            "basic"
        } else {
            "bearer"
        }
        .into(),
        r.class,
        if profiles::sms(b.profile) {
            BillingMetric::Requests
        } else {
            BillingMetric::VoiceSeconds
        },
        pricing,
        false,
    );
    // Reserve the entire bounded carrier liability before answering a call.
    if !profiles::sms(b.profile) {
        ctx.requested_voice_seconds = 1800;
        ctx.voice_initial_window = true;
    }
    Ok(ctx)
}
pub async fn reserve(
    state: &AppState,
    b: &Binding,
    r: &Resolved,
    id: &str,
) -> AppResult<MeteredProxyContext> {
    let ctx = context(state, b, r, id).await?;
    if r.class == CredentialClass::NyxidManagedMaster {
        state.billing.open_required(&ctx).await
    } else {
        state.billing.open(&ctx).await
    }
}
/// Compensate an effect that was never sent: a reserved, never-forwarded part
/// is released (settling it would match nothing) and a forwarded one settles
/// at zero. Every meter is attempted; anything left over stays unadmitted and
/// is released by the managed reservation sweep.
pub async fn cancel(state: &AppState, meters: &[MeteredProxyContext], reason: &str) {
    for meter in meters {
        let released = state.billing.fail(meter, reason).await;
        let settled = state
            .billing
            .settle(meter, PlatformUsage::default(), None, None)
            .await;
        if released.is_err() || settled.is_err() {
            tracing::warn!(
                reason,
                "Effect reservation compensation deferred to recovery"
            );
        }
    }
}
pub async fn settle(state: &AppState, meter: &MeteredProxyContext, seconds: i64) -> AppResult<()> {
    state
        .billing
        .settle(
            meter,
            PlatformUsage {
                requests: 1,
                voice_seconds: seconds.clamp(0, 1800),
                ..Default::default()
            },
            None,
            None,
        )
        .await
}

pub async fn settle_admitted(
    state: &AppState,
    id: &str,
    owner: &str,
    seconds: i64,
    requests: i64,
) -> AppResult<()> {
    crate::services::billing::meter::settle_channel_admission(
        &state.db,
        id,
        owner,
        PlatformUsage {
            requests,
            voice_seconds: seconds,
            ..Default::default()
        },
    )
    .await
}
