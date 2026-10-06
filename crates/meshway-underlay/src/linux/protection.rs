use meshway_authority::ExecutionSnapshot;
use serde::{Deserialize, Serialize};
use socket2::SockRef;
use std::{
    io,
    sync::{Arc, Mutex, RwLock, Weak},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use super::tasks::Tasks;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Mark {
    pub value: u32,
    pub mask: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Selector {
    pub value: u32,
    pub mask: u32,
}

impl Mark {
    pub fn transform(self, old: u32, selectors: &[Selector]) -> Result<u32> {
        if self.value == 0 || self.value & !self.mask != 0 {
            return Err(Error::Conflict);
        }
        let new = (old & !self.mask) | self.value;
        if selectors.iter().any(|s| {
            ((old & s.mask) == (s.value & s.mask)) != ((new & s.mask) == (s.value & s.mask))
        }) {
            return Err(Error::Conflict);
        }
        Ok(new)
    }
}

pub(super) struct Shared {
    pub mark: Mark,
    pub selectors: Vec<Selector>,
    pub conflict: CancellationToken,
    pub tasks: Tasks,
    pub sockets: Arc<Semaphore>,
    pub data_sockets: Arc<Semaphore>,
    pub attempts: Arc<Semaphore>,
    pub resolutions: Arc<Semaphore>,
    pub execution: RwLock<Option<(ExecutionSnapshot, CancellationToken)>>,
    pub resources: Resources,
}

pub(super) struct Resources {
    entries: Mutex<Vec<Weak<PhysicalResource>>>,
    limit: usize,
}
struct PhysicalResource {
    snapshot: Mutex<ExecutionSnapshot>,
    purpose: meshway_model::state::ResourceRef,
    stop: CancellationToken,
}
impl Resources {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            limit,
        }
    }
    pub fn handoff(&self, next: &ExecutionSnapshot) -> Result<()> {
        let mut entries = self.entries.lock().map_err(|_| Error::Worker)?;
        entries.retain(|entry| entry.strong_count() != 0);
        for entry in entries.iter().filter_map(Weak::upgrade) {
            let mut previous = entry.snapshot.lock().map_err(|_| Error::Worker)?;
            let compatible = !entry.stop.is_cancelled()
                && next.continuation_from(&previous).is_ok_and(|old| {
                    next.configuration()
                        .is_ok_and(|new| crate::resource_compatible(old, new, &entry.purpose))
                });
            if compatible {
                *previous = next.clone();
            } else {
                entry.stop.cancel();
            }
        }
        Ok(())
    }
}

impl Shared {
    pub fn check(&self) -> Result<()> {
        if self.conflict.is_cancelled() {
            return Err(Error::Conflict);
        }
        if self.tasks.stop.is_cancelled() {
            return Err(Error::Worker);
        }
        Ok(())
    }

    pub fn prepare(&self, socket: &SockRef<'_>) -> Result<()> {
        self.check()?;
        let old = socket.mark().map_err(|_| Error::Protection)?;
        let mark = self.mark.transform(old, &self.selectors)?;
        socket.set_mark(mark).map_err(|_| Error::Protection)?;
        if socket.mark().map_err(|_| Error::Protection)? != mark {
            return Err(Error::Conflict);
        }
        self.check()
    }

    pub fn suspend(&self) -> Result<()> {
        if let Some((_, token)) = self.execution.write().map_err(|_| Error::Worker)?.take() {
            token.cancel();
        }
        Ok(())
    }

    pub fn data(self: &Arc<Self>, snapshot: &ExecutionSnapshot) -> Result<Scope> {
        self.check()?;
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
        let active = self.execution.read().map_err(|_| Error::Worker)?;
        let (installed, token) = active.as_ref().ok_or(Error::Expired)?;
        if !installed.same_execution(snapshot) || token.is_cancelled() {
            return Err(Error::Expired);
        }
        Ok(Scope {
            shared: self.clone(),
            snapshot: Some(snapshot.clone()),
            epoch: token.clone(),
            physical: false,
            resource: None,
            binding: None,
        })
    }

    pub fn bootstrap(self: &Arc<Self>) -> Scope {
        Scope {
            shared: self.clone(),
            snapshot: None,
            epoch: self.tasks.stop.clone(),
            physical: false,
            resource: None,
            binding: None,
        }
    }
}

#[derive(Clone)]
pub(super) struct Scope {
    pub shared: Arc<Shared>,
    snapshot: Option<ExecutionSnapshot>,
    epoch: CancellationToken,
    physical: bool,
    resource: Option<meshway_model::state::ResourceRef>,
    binding: Option<Arc<PhysicalResource>>,
}

impl Scope {
    pub fn owner_current(&self) -> Result<()> {
        self.shared.check()?;
        if self.epoch.is_cancelled()
            || self
                .binding
                .as_ref()
                .is_some_and(|binding| binding.stop.is_cancelled())
        {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub fn for_resource(mut self, resource: meshway_model::state::ResourceRef) -> Self {
        self.resource = Some(resource);
        self
    }
    pub fn physical(mut self) -> Result<Self> {
        self.check()?;
        let snapshot = self.snapshot.clone().ok_or(Error::Target)?;
        let purpose = self.resource.clone().ok_or(Error::Target)?;
        // Serialize registration with Host's execution/resource handoff. A
        // handle minted before it cannot register an unchecked old descriptor
        // after the per-resource cancellation pass.
        let active = self.shared.execution.read().map_err(|_| Error::Worker)?;
        let (installed, _) = active.as_ref().ok_or(Error::Expired)?;
        if !installed.same_execution(&snapshot) || self.epoch.is_cancelled() {
            return Err(Error::Expired);
        }
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
        let mut entries = self
            .shared
            .resources
            .entries
            .lock()
            .map_err(|_| Error::Worker)?;
        entries.retain(|entry| entry.strong_count() != 0);
        if entries.len() >= self.shared.resources.limit {
            return Err(Error::Capacity);
        }
        let binding = Arc::new(PhysicalResource {
            snapshot: Mutex::new(snapshot),
            purpose,
            stop: self.epoch.child_token(),
        });
        entries.push(Arc::downgrade(&binding));
        drop(entries);
        drop(active);
        self.binding = Some(binding);
        self.physical = true;
        Ok(self)
    }
    pub fn frozen(&self) -> Result<Self> {
        let mut scope = self.clone();
        scope.snapshot = self.current()?;
        scope.physical = false;
        Ok(scope)
    }
    pub fn execution(&self) -> Result<ExecutionSnapshot> {
        self.current()?.ok_or(Error::Target)
    }
    pub fn check(&self) -> Result<()> {
        self.owner_current()?;
        if self.current().is_err() {
            return Err(Error::Expired);
        }
        Ok(())
    }

    fn current(&self) -> Result<Option<ExecutionSnapshot>> {
        let Some(original) = &self.snapshot else {
            return Ok(None);
        };
        let active = self.shared.execution.read().map_err(|_| Error::Worker)?;
        let (snapshot, token) = active.as_ref().ok_or(Error::Expired)?;
        if token.is_cancelled()
            || (!self.physical
                && (!snapshot.same_execution(original) || original.ensure_current().is_err()))
        {
            return Err(Error::Expired);
        }
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
        Ok(Some(snapshot.clone()))
    }

    pub fn prepare(&self, socket: &SockRef<'_>) -> io::Result<()> {
        self.prepare_checked(socket).map_err(io_error)
    }

    pub fn prepare_checked(&self, socket: &SockRef<'_>) -> Result<()> {
        self.check()?;
        self.shared.prepare(socket)?;
        self.check()
    }

    pub fn capacity(&self) -> Result<OwnedSemaphorePermit> {
        self.check()?;
        let capacity = if self.snapshot.is_some() {
            &self.shared.data_sockets
        } else {
            &self.shared.sockets
        };
        capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)
    }

    pub async fn invalidated(&self) {
        loop {
            if self.check().is_err() {
                return;
            }
            let Ok(current) = self.current() else {
                return;
            };
            tokio::select! { biased;
                _ = self.shared.conflict.cancelled() => return,
                _ = self.shared.tasks.stop.cancelled() => return,
                _ = self.epoch.cancelled() => return,
                _ = async { match &self.binding { Some(binding) => binding.stop.cancelled().await, None => std::future::pending().await } } => return,
                _ = async {
                    match current {
                        Some(snapshot) => snapshot.invalidated().await,
                        None => std::future::pending().await,
                    }
                } => {},
            }
            // A successful live same-target Host::install is an explicit
            // Control-issued carrier renewal. Queued packet guards retain their
            // own original deadlines; only this physical socket scope refreshes.
        }
    }

    /// Physical descriptors remain inactive during a publication gap. Only the
    /// Host's validated, compatible epoch handoff can supply current authority.
    /// Ordinary resolver/opening scopes keep their original invalidation rules.
    pub async fn closed(&self) {
        if !self.physical || self.snapshot.is_none() {
            return self.invalidated().await;
        }
        loop {
            if self.owner_current().is_err() {
                return;
            }
            let snapshot = self
                .shared
                .execution
                .read()
                .ok()
                .and_then(|active| active.as_ref().map(|(snapshot, _)| snapshot.clone()));
            let Some(snapshot) = snapshot else {
                return;
            };
            let Ok(deadline) = snapshot.retention_deadline() else {
                return;
            };
            tokio::select! { biased;
                _ = self.shared.conflict.cancelled() => return,
                _ = self.shared.tasks.stop.cancelled() => return,
                _ = self.epoch.cancelled() => return,
                _ = async { match &self.binding { Some(binding) => binding.stop.cancelled().await, None => std::future::pending().await } } => return,
                _ = tokio::time::sleep_until(deadline.into()) => {},
                _ = async {
                    if snapshot.ensure_current().is_ok() { snapshot.invalidated().await; }
                    else { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
                } => {},
            }
        }
    }
}

pub(super) fn io_error(error: Error) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_external_bits_and_rejects_changed_selector_results() {
        let mark = Mark {
            value: 0x100,
            mask: 0x100,
        };
        let external = Selector {
            value: 0x20,
            mask: 0xf0,
        };
        assert_eq!(mark.transform(0x20, &[external]).unwrap(), 0x120);
        let exact_zero = Selector {
            value: 0,
            mask: u32::MAX,
        };
        assert!(mark.transform(0, &[exact_zero]).is_err());
        // Testing only the usual zero mark would miss this socket's collision.
        let other = Selector {
            value: 0x120,
            mask: u32::MAX,
        };
        assert!(mark.transform(0, &[other]).is_ok());
        assert!(mark.transform(0x20, &[other]).is_err());
    }
}
