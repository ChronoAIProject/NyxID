//! Scoped, one-shot pause/failure points for real transaction race tests.
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};
use tokio::sync::Notify;
#[derive(Clone, Default)]
pub(crate) struct Gate {
    pub entered: Arc<Notify>,
    pub release: Arc<Notify>,
    pub fail: bool,
}
type Gates = Mutex<HashMap<(String, String, String), Gate>>;
static GATES: LazyLock<Gates> = LazyLock::new(Default::default);
pub(crate) fn install(db: &mongodb::Database, turn: &str, phase: &str, fail: bool) -> Gate {
    let gate = Gate {
        fail,
        ..Default::default()
    };
    GATES
        .lock()
        .unwrap()
        .insert((db.name().into(), turn.into(), phase.into()), gate.clone());
    gate
}
pub(crate) async fn reach(db: &mongodb::Database, turn: &str, phase: &str) -> bool {
    let gate = {
        GATES
            .lock()
            .unwrap()
            .remove(&(db.name().into(), turn.into(), phase.into()))
    };
    if let Some(gate) = gate {
        gate.entered.notify_one();
        gate.release.notified().await;
        gate.fail
    } else {
        false
    }
}
