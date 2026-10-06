//! Stop shares the persisted conversation fence and the signed node dispatcher.
use crate::{
    AppState,
    errors::AppResult,
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        machine_job::{COLLECTION_NAME as JOBS, MachineJob},
        node::{COLLECTION_NAME as NODES, Node},
    },
};
use futures::StreamExt;
use mongodb::{
    bson::{DateTime, doc},
    options::ReturnDocument,
};
use serde_json::json;
use std::{collections::HashSet, time::Duration};

pub async fn conversation(state: &AppState, user: &str, id: &str) -> AppResult<()> {
    let row = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one_and_update(
            doc! {"_id":id,"user_id":user,"active_turn":{"$ne":null}},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .return_document(ReturnDocument::After)
        .await?;
    let mut nodes: HashSet<String> = row
        .as_ref()
        .and_then(|r| r.active_turn.as_ref())
        .map(|t| t.machine_node_ids.iter().cloned().collect())
        .unwrap_or_default();
    let mut jobs = state
        .db
        .collection::<MachineJob>(JOBS)
        .find(doc! {"conversation_id":id,"user_id":user,"state":"running"})
        .await?;
    while let Some(job) = jobs.next().await {
        nodes.insert(job?.node_id);
    }
    state
        .db
        .collection::<MachineJob>(JOBS)
        .update_many(
            doc! {"conversation_id":id,"user_id":user,"state":"running"},
            doc! {"$set":{"state":"finished","finished_at":DateTime::now()}},
        )
        .await?;
    // Cancelling every job in this conversation also covers background jobs
    // started by an earlier turn. New turns have new node admission scopes.
    let args = json!({"conversation_id":id,"turn_id":row.as_ref().and_then(|r|r.active_turn.as_ref()).map(|t|t.turn_id.as_str()).unwrap_or_default()});
    futures::stream::iter(nodes)
        .for_each_concurrent(16, |node| {
            let args = args.clone();
            async move {
                send(state, &node, args).await;
            }
        })
        .await;
    Ok(())
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
    send(state, node, json!({"all":true,"scopes":scopes})).await;
    Ok(())
}

async fn send(state: &AppState, id: &str, args: serde_json::Value) {
    let work = async {
        if let Some(node) = state
            .db
            .collection::<Node>(NODES)
            .find_one(doc! {"_id":id})
            .await?
        {
            super::machine_tools::dispatch(state, &node, nyxid_machine::Operation::Cancel, args)
                .await?;
        }
        AppResult::Ok(())
    };
    if !tokio::time::timeout(Duration::from_millis(800), work)
        .await
        .is_ok_and(|r| r.is_ok())
    {
        tracing::warn!(
            node_id = id,
            "Machine stop delivery pending or node offline"
        );
    }
}
