//! Owned packet storage with explicit head/tail room and bounded owner pools.
use std::{
    fmt,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};
use zeroize::Zeroize;

pub const DEFAULT_HEADROOM: usize = 128;
pub const DEFAULT_TAILROOM: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BufferError {
    #[error("packet buffer bounds exceeded")]
    Bounds,
    #[error("invalid packet pool limits")]
    Limits,
    #[error("packet pool capacity exhausted")]
    Exhausted,
}

struct PoolState {
    free: Vec<Vec<u8>>,
    allocated: usize,
}

struct Pool {
    state: Mutex<PoolState>,
    limit: usize,
    capacity: usize,
    headroom: usize,
    payload_capacity: usize,
}

/// A per-owner pool. Outstanding buffers count towards capacity, including
/// buffers queued in another task. Exhaustion never allocates an overflow slot.
#[derive(Clone)]
pub struct PacketPool(Arc<Pool>);

impl PacketPool {
    pub fn new(
        limit: usize,
        payload_capacity: usize,
        headroom: usize,
        tailroom: usize,
    ) -> Result<Self, BufferError> {
        let capacity = headroom
            .checked_add(payload_capacity)
            .and_then(|n| n.checked_add(tailroom))
            .ok_or(BufferError::Limits)?;
        if limit == 0
            || limit > 4096
            || payload_capacity == 0
            || payload_capacity > crate::secure::MAX_FRAME
            || capacity
                .checked_mul(limit)
                .is_none_or(|n| n > 64 * 1024 * 1024)
        {
            return Err(BufferError::Limits);
        }
        Ok(Self(Arc::new(Pool {
            state: Mutex::new(PoolState {
                free: Vec::new(),
                allocated: 0,
            }),
            limit,
            capacity,
            headroom,
            payload_capacity,
        })))
    }

    pub fn acquire(&self, len: usize) -> Result<PacketBuffer, BufferError> {
        if len > self.0.payload_capacity {
            return Err(BufferError::Bounds);
        }
        let mut state = self.0.state.lock().map_err(|_| BufferError::Exhausted)?;
        let storage = if let Some(storage) = state.free.pop() {
            storage
        } else {
            if state.allocated == self.0.limit {
                return Err(BufferError::Exhausted);
            }
            state.allocated += 1;
            vec![0; self.0.capacity]
        };
        Ok(PacketBuffer {
            storage: Some(storage),
            head: self.0.headroom,
            len,
            dirty_start: self.0.headroom,
            dirty_end: self.0.headroom + len,
            pool: Some(self.0.clone()),
        })
    }
}

/// A single allocation carried through parsing, AEAD, framing and delivery.
/// Every region ever exposed as mutable is zeroized on drop, even after trim or
/// truncate. This covers cancellation, malformed plaintext and crypto failures.
pub struct PacketBuffer {
    storage: Option<Vec<u8>>,
    head: usize,
    len: usize,
    dirty_start: usize,
    dirty_end: usize,
    pool: Option<Arc<Pool>>,
}

impl PacketBuffer {
    pub fn payload(&self) -> &[u8] {
        &self.storage.as_ref().expect("owned storage")[self.head..self.head + self.len]
    }
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.storage.as_mut().expect("owned storage")[self.head..self.head + self.len]
    }
    pub fn prepend(&mut self, n: usize) -> Result<&mut [u8], BufferError> {
        self.head = self.head.checked_sub(n).ok_or(BufferError::Bounds)?;
        self.len += n;
        self.dirty_start = self.dirty_start.min(self.head);
        Ok(&mut self.payload_mut()[..n])
    }
    pub fn extend(&mut self, n: usize) -> Result<&mut [u8], BufferError> {
        let len = self
            .len
            .checked_add(n)
            .filter(|len| *len <= self.capacity() - self.head)
            .ok_or(BufferError::Bounds)?;
        let old = self.len;
        self.len = len;
        self.dirty_end = self.dirty_end.max(self.head + len);
        Ok(&mut self.payload_mut()[old..])
    }
    pub fn trim_front(&mut self, n: usize) {
        assert!(n <= self.len, "trim exceeds payload");
        self.head += n;
        self.len -= n;
    }
    pub fn truncate(&mut self, len: usize) {
        self.len = self.len.min(len);
    }
    pub fn capacity(&self) -> usize {
        self.storage.as_ref().expect("owned storage").len()
    }
    /// Compatibility boundary for Vec-based transport providers. This copies;
    /// the pooled allocation is zeroized and returned to its original owner.
    pub fn into_vec(self) -> Vec<u8> {
        self.payload().to_vec()
    }
}

impl From<Vec<u8>> for PacketBuffer {
    fn from(mut bytes: Vec<u8>) -> Self {
        let buffer = Self::from(bytes.as_slice());
        bytes.zeroize();
        buffer
    }
}
impl From<&[u8]> for PacketBuffer {
    fn from(bytes: &[u8]) -> Self {
        let mut storage = vec![0; DEFAULT_HEADROOM + bytes.len() + DEFAULT_TAILROOM];
        storage[DEFAULT_HEADROOM..DEFAULT_HEADROOM + bytes.len()].copy_from_slice(bytes);
        Self {
            storage: Some(storage),
            head: DEFAULT_HEADROOM,
            len: bytes.len(),
            dirty_start: DEFAULT_HEADROOM,
            dirty_end: DEFAULT_HEADROOM + bytes.len(),
            pool: None,
        }
    }
}
impl AsRef<[u8]> for PacketBuffer {
    fn as_ref(&self) -> &[u8] {
        self.payload()
    }
}
impl AsMut<[u8]> for PacketBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.payload_mut()
    }
}

impl meshway_transport::PayloadStorage for PacketBuffer {
    fn capacity(&self) -> usize {
        self.capacity()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send + Sync> {
        self
    }
}

impl From<meshway_transport::Payload> for PacketBuffer {
    fn from(payload: meshway_transport::Payload) -> Self {
        match payload {
            meshway_transport::Payload::Bytes(mut bytes) => {
                // Wire input already contains the header and authentication tags;
                // decoding reclaims that room, so no ingress headroom copy is needed.
                let len = bytes.len();
                bytes.resize(bytes.capacity(), 0);
                Self {
                    storage: Some(bytes),
                    head: 0,
                    len,
                    dirty_start: 0,
                    dirty_end: len,
                    pool: None,
                }
            }
            meshway_transport::Payload::Owned(bytes) => {
                // The only custom production payload is PacketBuffer. Other host
                // storage remains a supported, explicit compatibility boundary.
                if bytes.as_any().is::<Self>() {
                    match bytes.into_any().downcast::<Self>() {
                        Ok(buffer) => *buffer,
                        Err(_) => unreachable!("checked storage type"),
                    }
                } else {
                    Self::from(bytes.as_ref().as_ref())
                }
            }
        }
    }
}
impl Deref for PacketBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.payload()
    }
}
impl DerefMut for PacketBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.payload_mut()
    }
}
impl fmt::Debug for PacketBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PacketBuffer")
            .field("length", &self.len)
            .finish_non_exhaustive()
    }
}
impl Drop for PacketBuffer {
    fn drop(&mut self) {
        if let Some(mut storage) = self.storage.take() {
            storage[self.dirty_start..self.dirty_end].zeroize();
            if let Some(pool) = &self.pool
                && let Ok(mut state) = pool.state.lock()
            {
                state.free.push(storage);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_payload_retains_pool_allocation_and_capacity_until_completion_owner_drops() {
        let pool = PacketPool::new(1, 1280, DEFAULT_HEADROOM, DEFAULT_TAILROOM).unwrap();
        let packet = pool.acquire(32).unwrap();
        let pointer = packet.as_ptr();
        let capacity = packet.capacity();
        let frame = meshway_transport::Frame::owned(packet, None);
        assert_eq!(frame.payload.as_ptr(), pointer);
        assert_eq!(frame.payload.capacity(), capacity);
        assert!(matches!(pool.acquire(32), Err(BufferError::Exhausted)));
        let packet = PacketBuffer::from(frame.payload);
        assert_eq!(packet.as_ptr(), pointer);
        drop(packet);
        assert!(pool.acquire(32).is_ok());
        let wire = vec![7; 64];
        let pointer = wire.as_ptr();
        let packet = PacketBuffer::from(meshway_transport::Payload::from(wire));
        assert_eq!(packet.as_ptr(), pointer);
    }
    #[test]
    fn capacity_includes_outstanding_buffers_and_drop_zeroizes_trimmed_plaintext() {
        let pool = PacketPool::new(1, 1280, DEFAULT_HEADROOM, DEFAULT_TAILROOM).unwrap();
        let mut first = pool.acquire(1280).unwrap();
        let pointer = first.as_ptr();
        first.fill(0xa5);
        first.prepend(54).unwrap().fill(0xb6);
        first.extend(32).unwrap().fill(0xc7);
        assert!(matches!(pool.acquire(1), Err(BufferError::Exhausted)));
        first.trim_front(100);
        first.truncate(1);
        drop(first);
        let next = pool.acquire(1280).unwrap();
        assert_eq!(next.as_ptr(), pointer);
        assert!(next.storage.as_ref().unwrap().iter().all(|b| *b == 0));
    }
    #[test]
    fn bounds_fail_without_changing_active_bytes() {
        let pool = PacketPool::new(1, 2, 1, 1).unwrap();
        let mut packet = pool.acquire(2).unwrap();
        packet.copy_from_slice(&[1, 2]);
        assert_eq!(packet.prepend(2), Err(BufferError::Bounds));
        assert_eq!(packet.extend(2), Err(BufferError::Bounds));
        assert_eq!(packet.payload(), &[1, 2]);
        assert!(matches!(pool.acquire(3), Err(BufferError::Bounds)));
        assert!(PacketPool::new(usize::MAX, 1280, 128, 64).is_err());
    }
}
