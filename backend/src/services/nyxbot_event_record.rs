//! Read contexts for the independent encrypted fields retained on a NyxBot event.
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    models::nyxbot_channel::{ChannelTurnDelivery, EVENTS_COLLECTION_NAME, NyxbotEvent},
};
use zeroize::Zeroizing;

pub(crate) async fn event_context(
    keys: &EncryptionKeys,
    event: &NyxbotEvent,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let field = ["event_context_ciphertext", &event.channel_id];
    record_field::decrypt_context(
        keys,
        &RecordContext::new(EVENTS_COLLECTION_NAME, &event.id, &event.user_id, &field)?,
        event.event_context_version,
        ciphertext,
    )
    .await
}

pub(crate) async fn delivery_target(
    keys: &EncryptionKeys,
    event: &NyxbotEvent,
    delivery: &ChannelTurnDelivery,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    let field = ["delivery.target_ciphertext", &event.channel_id];
    record_field::decrypt_context(
        keys,
        &RecordContext::new(EVENTS_COLLECTION_NAME, &event.id, &event.user_id, &field)?,
        delivery.target_context_version,
        ciphertext,
    )
    .await
}

pub(crate) async fn guest_setup(
    keys: &EncryptionKeys,
    event: &bson::Document,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let invalid = || AppError::Internal("Guest queue unavailable".into());
    let field = [
        "guest_setup_payload",
        event.get_str("channel_id").map_err(|_| invalid())?,
    ];
    let context = RecordContext::new(
        EVENTS_COLLECTION_NAME,
        event.get_str("_id").map_err(|_| invalid())?,
        event.get_str("user_id").map_err(|_| invalid())?,
        &field,
    )?;
    let version = event
        .get("guest_setup_context_version")
        .cloned()
        .map(bson::from_bson::<Option<u8>>)
        .transpose()
        .map_err(|_| invalid())?
        .flatten();
    record_field::decrypt_context(
        keys,
        &context,
        version,
        event
            .get_binary_generic("guest_setup_payload")
            .map_err(|_| invalid())?,
    )
    .await
}
