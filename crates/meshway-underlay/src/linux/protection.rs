use meshway_engine::Desired;
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
    /// Ends this term of kernel ownership; [`Self::end`] records why.
    pub conflict: CancellationToken,
    pub ending: Arc<Ending>,
    pub network: super::kernel::network::Network,
    pub tasks: Tasks,
    pub sockets: Arc<Semaphore>,
    pub data_sockets: Arc<Semaphore>,
    pub attempts: Arc<Semaphore>,
    pub resolutions: Arc<Semaphore>,
    pub execution: RwLock<Option<Installed>>,
    pub resources: Resources,
    /// The kernel change an owner is retrying after a failed attempt.
    pub changes: tokio::sync::watch::Sender<Option<crate::KernelChangePending>>,
    /// The stopped exit role's cleanup its owner is retrying, live.
    pub cleanups: tokio::sync::watch::Sender<Option<crate::RetryPending>>,
}

/// What a Host publishes across its ownership terms: a re-proved term
/// continues the same channels, so their subscribers keep them.
#[derive(Default)]
pub(super) struct Published {
    pub network: super::kernel::network::Network,
    pub changes: tokio::sync::watch::Sender<Option<crate::KernelChangePending>>,
    pub cleanups: tokio::sync::watch::Sender<Option<crate::RetryPending>>,
}

/// The desired content capture is installed for. `session` ends every data
/// operation and registered descriptor (withdrawal, suspension); new content
/// ends only the descriptors whose exact purpose changed, never by content
/// generation.
pub(super) struct Installed {
    pub desired: Desired,
    pub session: CancellationToken,
}

impl Installed {
    pub fn new(desired: Desired, session: CancellationToken) -> Self {
        Self { desired, session }
    }
}

pub(super) struct Resources {
    entries: Mutex<Vec<Weak<PhysicalResource>>>,
    limit: usize,
}

/// A registered descriptor and its exact purpose. It stays open while every
/// applied content keeps that purpose unchanged.
struct PhysicalResource {
    purpose: ResourceRef,
    stop: CancellationToken,
}

impl Resources {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            limit,
        }
    }
    /// New content: keep every descriptor whose exact purpose is unchanged
    /// from `previous` to `next`; stop the others.
    pub fn apply(&self, previous: &Desired, next: &Desired) -> Result<()> {
        let mut entries = self.entries.lock().map_err(|_| Error::Worker)?;
        entries.retain(|entry| entry.strong_count() != 0);
        let same_self = previous.netmap().network_id == next.netmap().network_id
            && previous.netmap().node_id == next.netmap().node_id;
        for entry in entries.iter().filter_map(Weak::upgrade) {
            if !same_self
                || !crate::netmap_resource_compatible(
                    previous.netmap(),
                    next.netmap(),
                    &entry.purpose,
                )
            {
                entry.stop.cancel();
            }
        }
        Ok(())
    }
}

/// Why and when a term of kernel ownership ended. Created with the term's
/// token, before [`Shared`], so the term's own netlink connection tasks
/// record their end too.
#[derive(Default)]
pub(super) struct Ending(Mutex<Option<(crate::OwnershipEnd, std::time::Instant)>>);

impl Ending {
    /// End the term of `conflict` for `cause`. A proven conflict replaces an
    /// earlier unavailable end, never the reverse; a token cancelled without
    /// a recorded cause counts as a conflict.
    pub fn end(&self, conflict: &CancellationToken, cause: crate::OwnershipEnd) {
        if let Ok(mut ending) = self.0.lock() {
            let replace = match &*ending {
                None => true,
                Some((crate::OwnershipEnd::Unavailable { .. }, _)) => {
                    matches!(cause, crate::OwnershipEnd::Conflict { .. })
                }
                Some((crate::OwnershipEnd::Conflict { .. }, _)) => false,
            };
            if replace {
                *ending = Some((cause, std::time::Instant::now()));
            }
        }
        conflict.cancel();
    }
    fn recorded(&self) -> Option<(crate::OwnershipEnd, std::time::Instant)> {
        self.0.lock().ok().and_then(|ending| *ending)
    }
}

impl Shared {
    /// End this term of kernel ownership for `cause` (see [`Ending::end`]).
    pub fn end(&self, cause: crate::OwnershipEnd) {
        self.ending.end(&self.conflict, cause);
    }

    /// The channels a new term of this Host continues.
    pub fn published(&self) -> Published {
        let network = &self.network;
        Published {
            network: super::kernel::network::Network {
                changes: network.changes.clone(),
                dirty: network.dirty.clone(),
                control: network.control.clone(),
                // The remotes connectivity reported still hold in a new term.
                remotes: network.remotes.clone(),
                overlap: network.overlap.clone(),
                repairs: network.repairs.clone(),
                routes: None,
            },
            changes: self.changes.clone(),
            cleanups: self.cleanups.clone(),
        }
    }

    /// Why this term ended, and when; `None` while it holds.
    pub fn ended(&self) -> Option<(crate::OwnershipEnd, std::time::Instant)> {
        if !self.conflict.is_cancelled() {
            return None;
        }
        let recorded = self.ending.recorded();
        Some(recorded.unwrap_or((
            crate::OwnershipEnd::Conflict {
                check: "unrecorded",
            },
            std::time::Instant::now(),
        )))
    }

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

    /// Withdrawal: every operation and registered descriptor ends now.
    pub fn suspend(&self) -> Result<()> {
        if let Some(installed) = self.execution.write().map_err(|_| Error::Worker)?.take() {
            installed.session.cancel();
        }
        Ok(())
    }

    /// The installed content.
    pub fn installed(&self) -> Result<Option<Desired>> {
        Ok(self
            .execution
            .read()
            .map_err(|_| Error::Worker)?
            .as_ref()
            .map(|installed| installed.desired.clone()))
    }

    /// A data operation planned on `desired` while some content is installed.
    /// It ends at withdrawal; a descriptor it registers must keep the same
    /// purpose on the content installed at registration (see `physical`).
    pub fn data(self: &Arc<Self>, desired: &Desired) -> Result<ProtectionScope> {
        let mut scope = self.installed_data()?;
        scope.planned = Some(desired.clone());
        Ok(scope)
    }

    /// A data operation of whatever content is installed now.
    pub fn installed_data(self: &Arc<Self>) -> Result<ProtectionScope> {
        self.check()?;
        let active = self.execution.read().map_err(|_| Error::Worker)?;
        let installed = active.as_ref().ok_or(Error::Expired)?;
        Ok(ProtectionScope {
            shared: self.clone(),
            stop: installed.session.clone(),
            data: true,
            planned: None,
            purpose: None,
            resource: None,
        })
    }

    pub fn bootstrap(self: &Arc<Self>) -> ProtectionScope {
        ProtectionScope {
            shared: self.clone(),
            stop: self.tasks.stop.clone(),
            data: false,
            planned: None,
            purpose: None,
            resource: None,
        }
    }
}

/// Protection of one socket operation or descriptor: its single stop token
/// (Host lifetime for Server bootstrap, the installed session for an
/// operation, the registration for a descriptor) and its socket capacity.
#[derive(Clone)]
pub(super) struct ProtectionScope {
    pub shared: Arc<Shared>,
    stop: CancellationToken,
    /// Data sockets use the data capacity and require installed content.
    data: bool,
    /// The content the operation was planned on, if any.
    planned: Option<Desired>,
    /// The descriptor purpose this operation may register.
    purpose: Option<ResourceRef>,
    /// The registration that keeps a descriptor's entry in the Host's apply.
    resource: Option<Arc<PhysicalResource>>,
}

impl ProtectionScope {
    pub fn owner_current(&self) -> Result<()> {
        self.shared.check()?;
        if self.stop.is_cancelled() {
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub fn for_resource(mut self, purpose: ResourceRef) -> Self {
        self.purpose = Some(purpose);
        self
    }
    /// Register a descriptor for its purpose. Registration is serialized with
    /// the Host's apply, so it cannot outlive an incompatible content.
    pub fn physical(mut self) -> Result<Self> {
        self.check()?;
        let purpose = self.purpose.clone().ok_or(Error::Target)?;
        let active = self.shared.execution.read().map_err(|_| Error::Worker)?;
        let installed = active.as_ref().ok_or(Error::Expired)?;
        if self.stop.is_cancelled() {
            return Err(Error::Expired);
        }
        // Planned on earlier content: only a purpose the installed content
        // keeps unchanged may register, exactly as an apply would keep it.
        if let Some(planned) = &self.planned
            && planned.reference() != installed.desired.reference()
            && !crate::netmap_resource_compatible(
                planned.netmap(),
                installed.desired.netmap(),
                &purpose,
            )
        {
            return Err(Error::Expired);
        }
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
            purpose,
            stop: installed.session.child_token(),
        });
        entries.push(Arc::downgrade(&binding));
        drop(entries);
        drop(active);
        self.stop = binding.stop.clone();
        self.resource = Some(binding);
        Ok(self)
    }
    /// The installed content of a data scope.
    pub fn execution(&self) -> Result<Desired> {
        self.check()?;
        if !self.data {
            return Err(Error::Target);
        }
        self.shared.installed()?.ok_or(Error::Expired)
    }
    pub fn check(&self) -> Result<()> {
        self.owner_current()?;
        if self.data
            && self
                .shared
                .execution
                .read()
                .map_err(|_| Error::Worker)?
                .is_none()
        {
            return Err(Error::Expired);
        }
        Ok(())
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
        let capacity = if self.data {
            &self.shared.data_sockets
        } else {
            &self.shared.sockets
        };
        capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)
    }

    /// This scope's protection ended: conflict, Host stop, or its own token.
    pub async fn ended(&self) {
        tokio::select! { biased;
            _ = self.shared.conflict.cancelled() => {},
            _ = self.shared.tasks.stop.cancelled() => {},
            _ = self.stop.cancelled() => {},
        }
    }
}

pub(super) fn io_error(error: Error) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// X15: a term's request connection that stops ends it as unavailable
    /// with that cause recorded (not an unrecorded conflict); the first
    /// unavailable cause stands, and a proven conflict replaces it, never
    /// the reverse.
    #[test]
    fn a_stopped_connection_records_an_unavailable_end_a_conflict_replaces() {
        use crate::OwnershipEnd;
        let conflict = CancellationToken::new();
        let ending = Ending::default();
        ending.end(
            &conflict,
            OwnershipEnd::Unavailable {
                check: "connection_unavailable",
            },
        );
        assert!(conflict.is_cancelled());
        ending.end(
            &conflict,
            OwnershipEnd::Unavailable {
                check: "monitor_unavailable",
            },
        );
        assert_eq!(
            ending.recorded().map(|(cause, _)| cause),
            Some(OwnershipEnd::Unavailable {
                check: "connection_unavailable"
            }),
            "the first unavailable cause stands"
        );
        ending.end(&conflict, OwnershipEnd::Conflict { check: "monitor" });
        ending.end(
            &conflict,
            OwnershipEnd::Unavailable {
                check: "drain_unavailable",
            },
        );
        assert_eq!(
            ending.recorded().map(|(cause, _)| cause),
            Some(OwnershipEnd::Conflict { check: "monitor" })
        );
    }

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
