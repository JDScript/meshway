use crate::PacketBuffer;

mod sealed {
    pub trait Sealed {}
    impl Sealed for Vec<u8> {}
    impl Sealed for super::PacketBuffer {}
    impl Sealed for &[u8] {}
}

/// Packet parsing borrows slices without allocation, or retains owned buffer
/// storage for guarded delivery. Vec input is a compatibility copy into storage
/// with head/tail room; callers on the I/O path should supply PacketBuffer.
pub trait PacketInput: sealed::Sealed {
    type Storage: AsRef<[u8]>;
    fn into_storage(self) -> Self::Storage;
}

impl PacketInput for PacketBuffer {
    type Storage = Self;
    fn into_storage(self) -> Self {
        self
    }
}

impl PacketInput for Vec<u8> {
    type Storage = PacketBuffer;
    fn into_storage(self) -> PacketBuffer {
        self.into()
    }
}

impl<'a> PacketInput for &'a [u8] {
    type Storage = &'a [u8];
    fn into_storage(self) -> Self {
        self
    }
}
