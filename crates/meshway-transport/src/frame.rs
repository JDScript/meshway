use std::{
    any::Any,
    fmt,
    net::SocketAddr,
    ops::{Deref, DerefMut},
};

/// Host-owned storage retained without copying until completion. Capacity is
/// the full backing allocation, including inactive headroom and tailroom.
pub trait PayloadStorage: AsRef<[u8]> + AsMut<[u8]> + Send + Sync + 'static {
    fn capacity(&self) -> usize;
    fn as_any(&self) -> &dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync>;
}

pub enum Payload {
    Bytes(Vec<u8>),
    Owned(Box<dyn PayloadStorage>),
}

impl Payload {
    pub fn capacity(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.capacity(),
            Self::Owned(bytes) => bytes.capacity(),
        }
    }
    pub fn as_slice(&self) -> &[u8] {
        self
    }
    /// Explicit compatibility copy for custom storage. Vec input is moved.
    pub fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Bytes(bytes) => bytes,
            Self::Owned(bytes) => bytes.as_ref().as_ref().to_vec(),
        }
    }
}
impl From<Vec<u8>> for Payload {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Bytes(bytes)
    }
}
impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        self
    }
}
impl Deref for Payload {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Bytes(bytes) => bytes,
            Self::Owned(bytes) => bytes.as_ref().as_ref(),
        }
    }
}
impl DerefMut for Payload {
    fn deref_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Bytes(bytes) => bytes,
            Self::Owned(bytes) => bytes.as_mut().as_mut(),
        }
    }
}
impl<T: AsRef<[u8]> + ?Sized> PartialEq<T> for Payload {
    fn eq(&self, other: &T) -> bool {
        self.as_slice() == other.as_ref()
    }
}
impl Eq for Payload {}
impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Payload")
            .field("length", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

/// One opaque payload. For stream sessions `peer` must be absent or match the
/// session peer. Datagram sends require an explicit destination; receives carry
/// the actual source. No Mesh/IP/cryptographic header is interpreted here.
#[derive(Eq, PartialEq)]
pub struct Frame {
    pub payload: Payload,
    pub peer: Option<SocketAddr>,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("payload_len", &self.payload.len())
            .field("payload_capacity", &self.payload.capacity())
            .field("peer", &self.peer)
            .finish()
    }
}

impl Frame {
    pub fn new(payload: impl Into<Payload>, peer: Option<SocketAddr>) -> Self {
        Self {
            payload: payload.into(),
            peer,
        }
    }

    pub fn owned(payload: impl PayloadStorage, peer: Option<SocketAddr>) -> Self {
        Self {
            payload: Payload::Owned(Box::new(payload)),
            peer,
        }
    }
}
