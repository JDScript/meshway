//! Physical descriptor retention. Engine remains the sole authority owner.
use meshway_engine::Desired;
use meshway_model::state::ResourceRef;
use std::{
    io,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

pub(super) struct Resources {
    installed: Mutex<State>,
    capacity: Arc<Semaphore>,
}
#[derive(Default)]
struct State {
    desired: Option<Desired>,
    descriptors: Vec<Weak<Resource>>,
}
pub(super) struct Resource {
    pub purpose: ResourceRef,
    pub stop: CancellationToken,
    close: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    permit: Mutex<Option<OwnedSemaphorePermit>>,
}
impl Resource {
    pub fn check(&self) -> io::Result<()> {
        if self.stop.is_cancelled() {
            Err(io::ErrorKind::NotConnected.into())
        } else {
            Ok(())
        }
    }
    fn close(&self) {
        self.stop.cancel();
        if let Ok(mut close) = self.close.lock()
            && let Some(close) = close.take()
        {
            close();
        }
        if let Ok(mut permit) = self.permit.lock() {
            permit.take();
        }
    }
}
impl Drop for Resource {
    fn drop(&mut self) {
        self.close();
    }
}
impl Resources {
    pub fn new(capacity: usize) -> Self {
        Self {
            installed: Mutex::new(State::default()),
            capacity: Arc::new(Semaphore::new(capacity)),
        }
    }
    pub fn apply(&self, next: Option<Desired>) -> io::Result<()> {
        let mut state = self.installed.lock().map_err(|_| io::ErrorKind::Other)?;
        for resource in state.descriptors.iter().filter_map(Weak::upgrade) {
            let keep = state
                .desired
                .as_ref()
                .zip(next.as_ref())
                .is_some_and(|(old, new)| {
                    old.netmap().network_id == new.netmap().network_id
                        && old.netmap().node_id == new.netmap().node_id
                        && meshway_underlay::netmap_resource_compatible(
                            old.netmap(),
                            new.netmap(),
                            &resource.purpose,
                        )
                });
            if !keep {
                resource.close();
            }
        }
        state.descriptors.retain(|entry| {
            entry
                .upgrade()
                .is_some_and(|entry| !entry.stop.is_cancelled())
        });
        state.desired = next;
        Ok(())
    }
    pub fn execution(&self, resource: &Resource) -> io::Result<Desired> {
        resource.check()?;
        self.installed
            .lock()
            .map_err(|_| io::ErrorKind::Other)?
            .desired
            .clone()
            .ok_or(io::ErrorKind::NotConnected.into())
    }
    pub fn acquire(&self, desired: &Desired, purpose: ResourceRef) -> io::Result<Arc<Resource>> {
        let mut state = self.installed.lock().map_err(|_| io::ErrorKind::Other)?;
        if state
            .desired
            .as_ref()
            .is_none_or(|installed| !installed.same(desired))
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| io::ErrorKind::WouldBlock)?;
        let resource = Arc::new(Resource {
            purpose,
            stop: CancellationToken::new(),
            close: Mutex::new(None),
            permit: Mutex::new(Some(permit)),
        });
        state.descriptors.retain(|entry| {
            entry
                .upgrade()
                .is_some_and(|entry| !entry.stop.is_cancelled())
        });
        state.descriptors.push(Arc::downgrade(&resource));
        Ok(resource)
    }
}

struct Physical<T> {
    socket: Mutex<Option<T>>,
    read: futures_util::task::AtomicWaker,
    write: futures_util::task::AtomicWaker,
}
pub(super) struct Owned<T> {
    physical: Arc<Physical<T>>,
    pub resource: Arc<Resource>,
}
impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        self.resource.close();
    }
}

impl<T: Send + 'static> Owned<T> {
    pub fn new(socket: T, resource: Arc<Resource>) -> io::Result<Self> {
        let physical = Arc::new(Physical {
            socket: Mutex::new(Some(socket)),
            read: Default::default(),
            write: Default::default(),
        });
        let closing = physical.clone();
        let mut close = resource.close.lock().map_err(|_| io::ErrorKind::Other)?;
        resource.check()?;
        *close = Some(Box::new(move || {
            if let Ok(mut socket) = closing.socket.lock() {
                socket.take();
            }
            closing.read.wake();
            closing.write.wake();
        }));
        drop(close);
        Ok(Self { physical, resource })
    }
    pub fn with<R>(&self, call: impl FnOnce(&mut T) -> io::Result<R>) -> io::Result<R> {
        self.resource.check()?;
        let mut socket = self
            .physical
            .socket
            .lock()
            .map_err(|_| io::ErrorKind::Other)?;
        call(socket.as_mut().ok_or(io::ErrorKind::NotConnected)?)
    }
    pub fn poll<R>(
        &self,
        cx: &mut Context<'_>,
        write: bool,
        call: impl FnOnce(&mut T, &mut Context<'_>) -> Poll<io::Result<R>>,
    ) -> Poll<io::Result<R>> {
        if write {
            self.physical.write.register(cx.waker());
        } else {
            self.physical.read.register(cx.waker());
        }
        self.resource.check()?;
        let mut socket = self
            .physical
            .socket
            .lock()
            .map_err(|_| io::ErrorKind::Other)?;
        call(socket.as_mut().ok_or(io::ErrorKind::NotConnected)?, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn dropping_packet_owner_closes_descriptor_even_with_retained_authorization_context() {
        let capacity = Arc::new(Semaphore::new(1));
        let resource = Arc::new(Resource {
            purpose: ResourceRef::Listener { key: "test".into() },
            stop: CancellationToken::new(),
            close: Mutex::new(None),
            permit: Mutex::new(Some(capacity.clone().try_acquire_owned().unwrap())),
        });
        let authorization = resource.clone();
        let (socket, mut remote) = tokio::io::duplex(128);
        let owned = Owned::new(socket, resource).unwrap();
        assert_eq!(capacity.available_permits(), 0);
        drop(owned);
        assert!(authorization.check().is_err());
        assert_eq!(capacity.available_permits(), 1);
        assert_eq!(remote.read(&mut [0; 1]).await.unwrap(), 0);
    }
}
