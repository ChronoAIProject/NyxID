//! Observe completion of normal settlement hooks without changing their routing.
use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex},
};
use tokio::sync::watch;

type Completed = HashSet<String>;
static OBSERVERS: LazyLock<Mutex<HashMap<String, watch::Sender<Completed>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) struct Observer {
    database: String,
    receiver: watch::Receiver<Completed>,
}

pub(crate) fn observe(db: &mongodb::Database) -> Observer {
    let (sender, receiver) = watch::channel(Completed::new());
    assert!(
        OBSERVERS
            .lock()
            .unwrap()
            .insert(db.name().into(), sender)
            .is_none()
    );
    Observer {
        database: db.name().into(),
        receiver,
    }
}

pub(super) fn completed(db: &mongodb::Database, turn: &str) {
    if let Some(sender) = OBSERVERS.lock().unwrap().get(db.name()) {
        sender.send_modify(|turns| {
            turns.insert(turn.into());
        });
    }
}

impl Observer {
    pub(crate) async fn wait(&mut self, turn: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(25), async {
            loop {
                if self.receiver.borrow_and_update().contains(turn) {
                    return;
                }
                self.receiver.changed().await.unwrap();
            }
        })
        .await
        .expect("normal settlement delivery hooks must finish");
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        OBSERVERS.lock().unwrap().remove(&self.database);
    }
}
