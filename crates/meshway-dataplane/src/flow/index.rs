//! Derived lookup/expiry indexes; protocol state is owned only by FlowTable.
//! Each live entry has one tuple membership and one deadline per clock. Updating
//! or removing it removes the old deadlines, so traffic cannot grow stale heaps.
use super::{Flow, Key, Tuple};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

#[derive(Default)]
pub(super) struct Index {
    tuples: BTreeMap<Tuple, BTreeSet<Key>>,
    monotonic: BTreeSet<(Instant, Key)>,
    utc: BTreeSet<(u64, Key)>,
}

fn tuple(key: &Tuple) -> Tuple {
    key.clone().min(key.reversed())
}

fn utc(flow: &Flow) -> u64 {
    flow.permit_expires_at_ms.min(flow.guard_expires_at_ms)
}

impl Index {
    pub fn matching(&self, key: &Tuple) -> impl Iterator<Item = &Key> {
        self.tuples.get(&tuple(key)).into_iter().flatten()
    }

    pub fn expired(&self, now: Instant, now_ms: u64) -> Option<&Key> {
        self.monotonic
            .first()
            .filter(|(at, _)| *at <= now)
            .map(|(_, key)| key)
            .or_else(|| {
                self.utc
                    .first()
                    .filter(|(at, _)| *at <= now_ms)
                    .map(|(_, key)| key)
            })
    }

    pub fn insert(&mut self, key: &Key, flow: &Flow, previous: Option<&Flow>) {
        if let Some(previous) = previous {
            self.remove_deadlines(key, previous);
        } else {
            self.tuples
                .entry(tuple(&key.tuple))
                .or_default()
                .insert(key.clone());
        }
        self.monotonic.insert((flow.expires_at, key.clone()));
        self.utc.insert((utc(flow), key.clone()));
    }

    pub fn remove(&mut self, key: &Key, flow: &Flow) {
        self.remove_deadlines(key, flow);
        let tuple = tuple(&key.tuple);
        if let Some(keys) = self.tuples.get_mut(&tuple) {
            keys.remove(key);
            if keys.is_empty() {
                self.tuples.remove(&tuple);
            }
        }
    }

    fn remove_deadlines(&mut self, key: &Key, flow: &Flow) {
        self.monotonic.remove(&(flow.expires_at, key.clone()));
        self.utc.remove(&(utc(flow), key.clone()));
    }
}
