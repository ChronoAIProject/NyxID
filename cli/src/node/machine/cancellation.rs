//! Per-turn cancellation fences. A stopped turn cannot re-enter after its I/O
//! future has been dropped; a new turn in the same conversation gets a new key.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct Scope {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub turn_id: String,
}
struct Entry {
    stop: watch::Sender<bool>,
    touched: Instant,
}
#[derive(Default)]
pub struct Turns(Mutex<HashMap<Scope, Entry>>);
impl Turns {
    pub fn subscribe(&self, scope: &Scope) -> watch::Receiver<bool> {
        let mut turns = self.0.lock().expect("turn cancellation lock");
        // Signatures expire in two minutes. Keep stopped fences for at least
        // ten; live background scopes remain for their maximum job lifetime.
        turns.retain(|_, e| e.touched.elapsed() < Duration::from_secs(86460));
        if turns.len() >= 8192 && !turns.contains_key(scope) {
            // Do not evict a stopped fence while signed requests or jobs can
            // still exist. Reject new admission until retention expires.
            return watch::channel(true).1;
        }
        let entry = turns.entry(scope.clone()).or_insert_with(|| Entry {
            stop: watch::channel(false).0,
            touched: Instant::now(),
        });
        entry.touched = Instant::now();
        entry.stop.subscribe()
    }
    pub fn stop(&self, scope: Option<&Scope>) {
        if let Some(scope) = scope {
            let _ = self.subscribe(scope);
        }
        for (key, entry) in self.0.lock().expect("turn cancellation lock").iter_mut() {
            if scope.is_none_or(|s| {
                s.conversation_id == key.conversation_id
                    && (s.turn_id.is_empty() || s.turn_id == key.turn_id)
            }) {
                entry.stop.send_replace(true);
                entry.touched = Instant::now();
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_fences_late_calls_but_not_the_next_turn_or_another_chat() {
        let turns = Turns::default();
        let old = Scope {
            conversation_id: "chat".into(),
            turn_id: "old".into(),
        };
        let waiting = turns.subscribe(&old);
        let other = Scope {
            conversation_id: "other".into(),
            ..old.clone()
        };
        let other_rx = turns.subscribe(&other);
        turns.stop(Some(&old));
        assert!(*waiting.borrow());
        assert!(*turns.subscribe(&old).borrow());
        assert!(!*other_rx.borrow());
        assert!(
            !*turns
                .subscribe(&Scope {
                    turn_id: "new".into(),
                    ..old
                })
                .borrow()
        );
        turns.stop(None);
        assert!(*other_rx.borrow());
    }
}
