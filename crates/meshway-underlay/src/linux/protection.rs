use meshway_authority::ExecutionSnapshot;
use meshway_model::state::ResourceRef;
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
    pub network_changes: tokio::sync::watch::Sender<u64>,
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

/// A registered descriptor and the installed target it currently follows.
struct PhysicalResource {
    snapshot: Mutex<ExecutionSnapshot>,
    purpose: ResourceRef,
    stop: CancellationToken,
}

impl PhysicalResource {
    fn snapshot(&self) -> Option<ExecutionSnapshot> {
        self.snapshot.lock().ok().map(|snapshot| snapshot.clone())
    }
}

/// Interchangeable installed targets: the same netmap content and lease cell
/// (any instance binding), or the same legacy execution.
pub(super) fn same_target(a: &ExecutionSnapshot, b: &ExecutionSnapshot) -> bool {
    if a.is_netmap() {
        a.same_netmap(b)
    } else {
        a.same_execution(b)
    }
}

/// Whether a descriptor created under `previous` may follow `next`: the same
/// target, or a Control-validated continuation whose exact resource purpose is
/// unchanged. Any other replacement stops it.
fn follows(previous: &ExecutionSnapshot, next: &ExecutionSnapshot, purpose: &ResourceRef) -> bool {
    if same_target(previous, next) {
        return true;
    }
    if next.is_netmap() {
        next.netmap_continuation_from(previous).is_ok_and(|old| {
            next.netmap().is_ok_and(|new| {
                crate::netmap_resource_compatible(&old.netmap, &new.netmap, purpose)
            })
        })
    } else {
        next.continuation_from(previous).is_ok_and(|old| {
            next.configuration()
                .is_ok_and(|new| crate::resource_compatible(old, new, purpose))
        })
    }
}

impl Resources {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            limit,
        }
    }
    /// Rebind every compatible descriptor to `next`; stop the others. Same
    /// content (lease renewal or instance change) rebinds everything.
    pub fn handoff(&self, next: &ExecutionSnapshot) -> Result<()> {
        let mut entries = self.entries.lock().map_err(|_| Error::Worker)?;
        entries.retain(|entry| entry.strong_count() != 0);
        for entry in entries.iter().filter_map(Weak::upgrade) {
            let mut previous = entry.snapshot.lock().map_err(|_| Error::Worker)?;
            if !entry.stop.is_cancelled() && follows(&previous, next, &entry.purpose) {
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

    /// The installed target, whether or not it is still current.
    pub fn installed(&self) -> Result<Option<ExecutionSnapshot>> {
        Ok(self
            .execution
            .read()
            .map_err(|_| Error::Worker)?
            .as_ref()
            .filter(|(_, token)| !token.is_cancelled())
            .map(|(snapshot, _)| snapshot.clone()))
    }

    pub fn data(self: &Arc<Self>, snapshot: &ExecutionSnapshot) -> Result<ProtectionScope> {
        self.check()?;
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
        let active = self.execution.read().map_err(|_| Error::Worker)?;
        let (installed, token) = active.as_ref().ok_or(Error::Expired)?;
        if !same_target(installed, snapshot) || token.is_cancelled() {
            return Err(Error::Expired);
        }
        Ok(ProtectionScope {
            shared: self.clone(),
            epoch: token.clone(),
            lifetime: Lifetime::Operation(Box::new(Operation {
                snapshot: snapshot.clone(),
                resource: None,
            })),
        })
    }

    pub fn bootstrap(self: &Arc<Self>) -> ProtectionScope {
        ProtectionScope {
            shared: self.clone(),
            epoch: self.tasks.stop.clone(),
            lifetime: Lifetime::Bootstrap,
        }
    }
}

/// What bounds a scope. Exactly one applies; there is no mixed state.
#[derive(Clone)]
enum Lifetime {
    /// Server bootstrap: no execution target, Host-lifetime epoch, bootstrap
    /// socket capacity.
    Bootstrap,
    /// One operation under its original snapshot. A same-content netmap
    /// renewal keeps it current; any other replacement ends it for good.
    Operation(Box<Operation>),
    /// A registered descriptor. It follows compatible replacements of the
    /// installed target and stops on incompatible ones, Host suspension, or
    /// the end of its target's lease.
    Resource(Arc<PhysicalResource>),
}

#[derive(Clone)]
struct Operation {
    snapshot: ExecutionSnapshot,
    /// The descriptor purpose this operation may register.
    resource: Option<ResourceRef>,
}

#[derive(Clone)]
pub(super) struct ProtectionScope {
    pub shared: Arc<Shared>,
    epoch: CancellationToken,
    lifetime: Lifetime,
}

impl ProtectionScope {
    pub fn owner_current(&self) -> Result<()> {
        self.shared.check()?;
        if self.epoch.is_cancelled()
            || matches!(&self.lifetime, Lifetime::Resource(binding) if binding.stop.is_cancelled())
        {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub fn for_resource(mut self, purpose: ResourceRef) -> Self {
        if let Lifetime::Operation(operation) = &mut self.lifetime {
            operation.resource = Some(purpose);
        }
        self
    }
    pub fn physical(mut self) -> Result<Self> {
        self.check()?;
        let Lifetime::Operation(operation) = &self.lifetime else {
            return Err(Error::Target);
        };
        let (snapshot, purpose) = match &**operation {
            Operation {
                snapshot,
                resource: Some(purpose),
            } => (snapshot, purpose),
            _ => return Err(Error::Target),
        };
        // Serialize registration with Host's execution/resource handoff. A
        // handle minted before it cannot register an unchecked old descriptor
        // after the per-resource cancellation pass.
        let active = self.shared.execution.read().map_err(|_| Error::Worker)?;
        let (installed, _) = active.as_ref().ok_or(Error::Expired)?;
        if !same_target(installed, snapshot) || self.epoch.is_cancelled() {
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
            snapshot: Mutex::new(installed.clone()),
            purpose: purpose.clone(),
            stop: self.epoch.child_token(),
        });
        entries.push(Arc::downgrade(&binding));
        drop(entries);
        drop(active);
        self.lifetime = Lifetime::Resource(binding);
        Ok(self)
    }
    /// An operation scope fixed at the currently installed target. A later
    /// replacement ends it even when the descriptor itself is retained.
    pub fn frozen(&self) -> Result<Self> {
        let mut scope = self.clone();
        if let Some(snapshot) = self.current()? {
            scope.lifetime = Lifetime::Operation(Box::new(Operation {
                snapshot,
                resource: None,
            }));
        }
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
        if matches!(self.lifetime, Lifetime::Bootstrap) {
            return Ok(None);
        }
        let active = self.shared.execution.read().map_err(|_| Error::Worker)?;
        let (installed, token) = active.as_ref().ok_or(Error::Expired)?;
        if token.is_cancelled() {
            return Err(Error::Expired);
        }
        if let Lifetime::Operation(operation) = &self.lifetime
            && (!same_target(installed, &operation.snapshot)
                || operation.snapshot.ensure_current().is_err())
        {
            return Err(Error::Expired);
        }
        installed.ensure_current().map_err(|_| Error::Expired)?;
        Ok(Some(installed.clone()))
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
        let capacity = if matches!(self.lifetime, Lifetime::Bootstrap) {
            &self.shared.sockets
        } else {
            &self.shared.data_sockets
        };
        capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)
    }

    async fn stopped(&self) {
        tokio::select! { biased;
            _ = self.shared.conflict.cancelled() => {},
            _ = self.shared.tasks.stop.cancelled() => {},
            _ = self.epoch.cancelled() => {},
            _ = async { match &self.lifetime {
                Lifetime::Resource(binding) => binding.stop.cancelled().await,
                _ => std::future::pending().await,
            } } => {},
        }
    }

    /// I/O authority ended. A same-content renewal (or an explicit compatible
    /// Host replacement for a descriptor) does not end it.
    pub async fn invalidated(&self) {
        loop {
            let Ok(current) = self.current() else {
                return;
            };
            if self.owner_current().is_err() {
                return;
            }
            tokio::select! { biased;
                _ = self.stopped() => return,
                _ = async {
                    match current {
                        Some(snapshot) => snapshot.invalidated().await,
                        None => std::future::pending().await,
                    }
                } => {},
            }
        }
    }

    /// Descriptor lifetime: the resource is stopped by the Host (incompatible
    /// replacement, suspension, shutdown) or its target's lease has ended.
    /// Between a replacement's invalidation and the Host's handoff the
    /// descriptor stays open without I/O authority. Other scopes close with
    /// their I/O authority.
    pub async fn closed(&self) {
        let Lifetime::Resource(binding) = &self.lifetime else {
            return self.invalidated().await;
        };
        loop {
            if self.owner_current().is_err() {
                return;
            }
            let Some(deadline) = binding.snapshot().map(|snapshot| snapshot.deadline()) else {
                return;
            };
            if !deadline.valid() {
                return;
            }
            tokio::select! { biased;
                _ = self.stopped() => return,
                // Renewal or rebinding may have extended it; read again.
                _ = deadline.ended(&binding.stop) => {},
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
