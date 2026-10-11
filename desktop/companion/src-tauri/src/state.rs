use std::sync::{Mutex, MutexGuard};

use crate::model::CompanionSnapshot;
use crate::persistence::SnapshotStore;
use crate::scheduler::{CompanionEngine, SchedulerError};

pub struct CompanionState {
    snapshot: Mutex<CompanionSnapshot>,
    transaction: Mutex<()>,
    store: SnapshotStore,
}

pub struct StateUpdate<T> {
    pub snapshot: CompanionSnapshot,
    pub value: T,
    pub changed: bool,
}

impl CompanionState {
    pub fn new(snapshot: CompanionSnapshot, store: SnapshotStore) -> Self {
        Self {
            snapshot: Mutex::new(snapshot),
            transaction: Mutex::new(()),
            store,
        }
    }

    pub fn snapshot(&self) -> Result<CompanionSnapshot, String> {
        Ok(self.lock()?.clone())
    }

    pub fn update<T>(
        &self,
        operation: impl FnOnce(&CompanionSnapshot, &mut CompanionEngine) -> Result<T, SchedulerError>,
        effect: impl FnOnce(&StateUpdate<T>) -> Result<(), String>,
    ) -> Result<StateUpdate<T>, String> {
        let _transaction = self
            .transaction
            .lock()
            .map_err(|_| "companion transaction lock is poisoned".to_owned())?;
        let mut guard = self.lock()?;
        let mut engine = CompanionEngine::new(guard.clone()).map_err(|error| error.to_string())?;
        let value = operation(&guard, &mut engine).map_err(|error| error.to_string())?;
        let candidate = engine.into_snapshot();
        candidate.validate().map_err(|error| error.to_string())?;
        let changed = candidate != *guard;
        if changed {
            self.store
                .save(&candidate)
                .map_err(|error| error.to_string())?;
            *guard = candidate.clone();
        }

        let update = StateUpdate {
            snapshot: candidate,
            value,
            changed,
        };
        drop(guard);

        effect(&update)?;
        Ok(update)
    }

    fn lock(&self) -> Result<MutexGuard<'_, CompanionSnapshot>, String> {
        self.snapshot
            .lock()
            .map_err(|_| "companion state lock is poisoned".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;

    use super::*;
    use crate::model::MealId;

    #[test]
    fn failed_updates_do_not_change_or_persist_state() {
        let directory = tempfile::tempdir().unwrap();
        let store = SnapshotStore::in_directory(directory.path());
        let snapshot = CompanionSnapshot::default();
        store.save(&snapshot).unwrap();
        let state = CompanionState::new(snapshot.clone(), store.clone());

        let result = state.update(
            |_, engine| {
                engine.snooze(MealId::Breakfast, 0, chrono::Utc::now())?;
                Ok(())
            },
            |_| Ok(()),
        );

        assert!(result.is_err());
        assert_eq!(state.snapshot().unwrap(), snapshot);
        assert_eq!(store.load().unwrap(), Some(snapshot));
    }

    #[test]
    fn serializes_effects_with_the_mutation_that_produced_them() {
        let directory = tempfile::tempdir().unwrap();
        let store = SnapshotStore::in_directory(directory.path());
        let snapshot = CompanionSnapshot::default();
        store.save(&snapshot).unwrap();
        let state = Arc::new(CompanionState::new(snapshot, store));
        let effects = Arc::new(Mutex::new(Vec::new()));
        let (first_effect_started_tx, first_effect_started_rx) = mpsc::channel();
        let (release_first_effect_tx, release_first_effect_rx) = mpsc::channel();
        let (second_mutation_started_tx, second_mutation_started_rx) = mpsc::channel();

        let first_state = Arc::clone(&state);
        let first_effects = Arc::clone(&effects);
        let first = thread::spawn(move || {
            first_state
                .update(
                    |_, engine| {
                        engine.set_quiet_mode(true);
                        Ok(())
                    },
                    |_| {
                        first_effects.lock().unwrap().push("first-started");
                        first_effect_started_tx.send(()).unwrap();
                        release_first_effect_rx.recv().unwrap();
                        first_effects.lock().unwrap().push("first-finished");
                        Ok(())
                    },
                )
                .unwrap();
        });

        first_effect_started_rx.recv().unwrap();

        let second_state = Arc::clone(&state);
        let second_effects = Arc::clone(&effects);
        let second = thread::spawn(move || {
            second_mutation_started_tx.send(()).unwrap();
            second_state
                .update(
                    |_, engine| {
                        engine.set_quiet_mode(false);
                        Ok(())
                    },
                    |_| {
                        second_effects.lock().unwrap().push("second");
                        Ok(())
                    },
                )
                .unwrap();
        });

        second_mutation_started_rx.recv().unwrap();
        assert_eq!(*effects.lock().unwrap(), ["first-started"]);

        release_first_effect_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();

        assert_eq!(
            *effects.lock().unwrap(),
            ["first-started", "first-finished", "second"]
        );
        assert!(!state.snapshot().unwrap().settings.quiet_mode);
    }
}
