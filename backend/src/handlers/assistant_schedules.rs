//! NyxBot-only schedule tools. They share trigger validation and persistence.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        trigger::{TriggerDelivery, TriggerStatus, TriggerVerification},
        trigger_schedule::{OverlapPolicy, TriggerSource},
    },
    services::{assistant_team_service as team, trigger_schedule as schedules, trigger_service},
};
use serde_json::{Value, json};

async fn agent(
    state: &AppState,
    owner: &str,
    name: Option<&str>,
) -> AppResult<crate::models::assistant_agent::AssistantAgent> {
    match name {
        None | Some("nyxbot") => team::ensure_nyxbot(&state.db, owner).await,
        Some(name) => team::live_specialist(&state.db, owner, name).await,
    }
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> AppResult<T> {
    serde_json::from_value(v.clone())
        .map_err(|_| AppError::ValidationError("Invalid automation specification".into()))
}

pub(crate) async fn dispatch(
    state: &AppState,
    owner: &str,
    name: &str,
    args: &Value,
) -> AppResult<Value> {
    let mut settings = crate::services::assistant_settings_service::get(&state.db, owner).await?;
    if name == "create_schedule"
        && let Some(zone) = args["owner_timezone"].as_str()
    {
        let before = settings.clone();
        settings = crate::services::assistant_settings_service::update(
            &state.db,
            owner,
            crate::services::assistant_settings_service::Update {
                timezone: Some(zone.into()),
                ..Default::default()
            },
        )
        .await?;
        crate::services::assistant_settings_service::audit(&state.db, owner, &before, &settings)
            .await;
    }
    let zone = settings.timezone.as_deref().unwrap_or("UTC");
    if name == "list_schedules" {
        let rows = trigger_service::list_for_owner(&state.db, owner).await?;
        return Ok(json!({
            "timezone": settings.timezone,
            "schedules": rows.into_iter().filter(|t|t.source==TriggerSource::Schedule || matches!(t.delivery, TriggerDelivery::Assistant { .. })).map(|t|super::triggers::response(state,t,zone)).collect::<Vec<_>>(),
        }));
    }
    if name == "create_schedule" {
        if settings.timezone.is_none() {
            return Err(AppError::ValidationError("Ask the owner for their IANA timezone and save it in NyxBot settings before scheduling".into()));
        }
        let target = agent(state, owner, args["agent"].as_str()).await?;
        let delivery = TriggerDelivery::Assistant {
            confirmation_policy: args
                .get("confirmation_policy")
                .map(parse)
                .transpose()?
                .unwrap_or_default(),
            agent_id: target.id,
            thread_policy: args.get("thread_policy").map(parse).transpose()?,
            instruction: args["instruction"].as_str().unwrap_or_default().into(),
            deliver_to: args
                .get("deliver_to")
                .map(parse)
                .transpose()?
                .unwrap_or_default(),
        };
        super::trigger_scheduler::validate_delivery(state, owner, &delivery).await?;
        let created = trigger_service::create(
            &state.db,
            &state.encryption_keys,
            trigger_service::CreateInput {
                user_id: owner.into(),
                label: args["label"].as_str().unwrap_or_default().into(),
                user_service_id: None,
                source: TriggerSource::Schedule,
                setup_watch_id: None,
                schedule: Some(
                    parse::<super::trigger_schedule_dto::ScheduleDto>(&args["schedule"])?.into(),
                ),
                overlap: args
                    .get("overlap")
                    .map(parse)
                    .transpose()?
                    .unwrap_or_default(),
                verification: TriggerVerification::Schedule,
                delivery,
            },
        )
        .await?;
        schedules::audit(
            &state.db,
            owner,
            "trigger_created",
            mongodb::bson::doc! {
                "trigger_id": &created.trigger.id,
                "source": "schedule",
                "delivery_type": "assistant",
            },
        )
        .await;
        return Ok(json!({
            "schedule": super::triggers::response(state,created.trigger,zone),
            "timezone": zone,
        }));
    }
    let id = args["id"].as_str().unwrap_or_default();
    let current = trigger_service::ensure_actor_can_write(&state.db, owner, id).await?;
    if current.user_id != owner
        || (current.source != TriggerSource::Schedule
            && !(name == "update_schedule"
                && matches!(current.delivery, TriggerDelivery::Assistant { .. })))
    {
        return Err(AppError::TriggerNotFound);
    }
    match name {
        "delete_schedule" => {
            trigger_service::delete(&state.db, &current).await?;
            schedules::audit(
                &state.db,
                owner,
                "trigger_deleted",
                mongodb::bson::doc! { "trigger_id": id },
            )
            .await;
            Ok(json!({ "deleted": id }))
        }
        "run_schedule_now" => Ok(json!({
            "run": super::triggers::run_response(schedules::enqueue_now(&state.db,&current).await?),
        })),
        "update_schedule" => {
            let mut delivery = current.delivery.clone();
            if let TriggerDelivery::Assistant {
                agent_id,
                thread_policy,
                instruction,
                deliver_to,
                confirmation_policy,
            } = &mut delivery
            {
                if let Some(name) = args["agent"].as_str() {
                    *agent_id = agent(state, owner, Some(name)).await?.id;
                }
                if let Some(v) = args.get("thread_policy") {
                    *thread_policy = Some(parse(v)?);
                }
                if let Some(v) = args["instruction"].as_str() {
                    *instruction = v.into();
                }
                if let Some(v) = args.get("confirmation_policy") {
                    *confirmation_policy = parse(v)?;
                }
                if let Some(v) = args.get("deliver_to") {
                    *deliver_to = parse(v)?;
                }
            }
            super::trigger_scheduler::validate_delivery(state, owner, &delivery).await?;
            let status = args["paused"].as_bool().map(|v| {
                if v {
                    TriggerStatus::Disabled
                } else {
                    TriggerStatus::Active
                }
            });
            let updated = trigger_service::update(
                &state.db,
                &state.encryption_keys,
                &current,
                trigger_service::UpdateInput {
                    label: args["label"].as_str().map(str::to_owned),
                    status,
                    delivery: Some(delivery),
                    schedule: args
                        .get("schedule")
                        .map(parse::<super::trigger_schedule_dto::ScheduleDto>)
                        .transpose()?
                        .map(Into::into),
                    overlap: args
                        .get("overlap")
                        .map(parse::<OverlapPolicy>)
                        .transpose()?,
                },
            )
            .await?;
            schedules::audit(
                &state.db,
                owner,
                match status {
                    Some(TriggerStatus::Disabled) => "trigger_paused",
                    Some(TriggerStatus::Active) => "trigger_resumed",
                    None => "trigger_updated",
                },
                mongodb::bson::doc! { "trigger_id": id },
            )
            .await;
            Ok(json!({
                "schedule": super::triggers::response(state,updated.trigger,zone),
                "timezone": zone,
            }))
        }
        _ => Err(AppError::TriggerNotFound),
    }
}
