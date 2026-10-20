//! Stop shares the persisted conversation fence and the signed node dispatcher.
use crate::{
    AppState,
    errors::AppResult,
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        machine_job::{COLLECTION_NAME as JOBS, MachineJob},
    },
};
use futures::StreamExt;
use mongodb::bson::{DateTime, doc};
use serde_json::json;

pub async fn conversation(state: &AppState, user: &str, id: &str) -> AppResult<()> {
    crate::services::machine_service::cancel_conversation(state, user, id).await
}

pub async fn machine(state: &AppState, node: &str) -> AppResult<()> {
    state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .update_many(
            doc! {"active_turn.machine_node_ids":node},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await?;
    state
        .db
        .collection::<MachineJob>(JOBS)
        .update_many(
            doc! {"node_id":node,"state":"running"},
            doc! {"$set":{"state":"finished","finished_at":DateTime::now()}},
        )
        .await?;
    let mut stopped = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"active_turn.machine_node_ids":node,"active_turn.stop_requested":true})
        .limit(1024)
        .await?;
    let mut scopes = Vec::new();
    while let Some(row) = stopped.next().await {
        let row = row?;
        if let Some(turn) = row.active_turn {
            scopes.push(json!({"conversation_id":row.id,"turn_id":turn.turn_id}));
        }
    }
    crate::services::machine_service::send_cancel(state, node, json!({"all":true,"scopes":scopes}))
        .await;
    Ok(())
}
