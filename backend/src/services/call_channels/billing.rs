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
    let owner = state
        .billing
        .owner_resolver()
        .resolve_for_execution(&b.user_id, &r.resource_owner, r.class)
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
