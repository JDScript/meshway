//! Undo for observation mutations inside the SQLite transaction. Unchanged
//! tasks never enter this map; retained measurement indexes remain shared.
use super::{Observations, StoredTask};
use meshway_model::{NetworkId, ObservationId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct ObservationUndo {
    tasks: BTreeMap<ObservationId, Option<StoredTask>>,
}
impl Observations {
    pub(super) fn begin_changes(&mut self) {
        assert!(self.undo.is_none(), "nested observation staging");
        self.undo = Some(ObservationUndo::default());
    }
    fn remember(&mut self, id: ObservationId) {
        if let Some(undo) = &mut self.undo {
            undo.tasks
                .entry(id)
                .or_insert_with(|| self.tasks.get(&id).cloned());
        }
    }
    pub(super) fn task_mut(&mut self, id: ObservationId) -> Option<&mut StoredTask> {
        self.remember(id);
        self.tasks.get_mut(&id)
    }
    pub(super) fn insert_task(&mut self, id: ObservationId, task: StoredTask) {
        self.remember(id);
        self.tasks.insert(id, task);
    }
    pub(super) fn remove_task(&mut self, id: ObservationId) {
        self.remember(id);
        self.tasks.remove(&id);
    }
    pub(super) fn rollback_changes(&mut self) {
        if let Some(undo) = self.undo.take() {
            for (id, previous) in undo.tasks {
                match previous {
                    Some(task) => {
                        self.tasks.insert(id, task);
                    }
                    None => {
                        self.tasks.remove(&id);
                    }
                }
            }
        }
    }
    pub(super) fn commit_changes(&mut self) -> BTreeSet<NetworkId> {
        let mut changed = BTreeSet::new();
        if let Some(undo) = self.undo.take() {
            for (id, previous) in undo.tasks {
                match (previous, self.tasks.get(&id)) {
                    (Some(old), Some(current)) if !progress_changed(&old, current) => {}
                    (_, Some(current)) => {
                        changed.insert(current.network);
                    }
                    (Some(old), None) => {
                        changed.insert(old.network);
                    }
                    (None, None) => {}
                }
            }
        }
        changed
    }
}
fn progress_changed(old: &StoredTask, current: &StoredTask) -> bool {
    old.task.netmap != current.task.netmap
        || old.task.phase != current.task.phase
        || old.task.expires_at != current.task.expires_at
        || old.subscribers.len() != current.subscribers.len()
        || old.subscribers.iter().any(|(id, old)| {
            current
                .subscribers
                .get(id)
                .is_none_or(|s| s.expires_at != old.expires_at || s.terminal != old.terminal)
        })
}
