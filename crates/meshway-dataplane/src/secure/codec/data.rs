//! Borrowed v5 Data decoding. Nothing here reconstructs an execution context or
//! authenticates a CID; the caller must look it up in its original circuit owner.
use super::{Error, MAGIC, MAX_FRAME, MAX_PLAINTEXT};

pub(super) const HEADER: usize = 54;

pub(super) struct DataView<'a> {
    pub circuit: &'a [u8; 32],
    pub reply: bool,
    pub sequence: u64,
    pub body: &'a [u8],
    pub tag: [u8; 16],
}

impl<'a> DataView<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_FRAME
            || bytes.len() < HEADER + 32
            || &bytes[..8] != MAGIC
            || bytes[8] != 3
            || bytes[9] & !1 != 0
        {
            return Err(Error::Wire);
        }
        let sequence = u64::from_be_bytes(bytes[42..50].try_into().map_err(|_| Error::Wire)?);
        let length =
            u32::from_be_bytes(bytes[50..54].try_into().map_err(|_| Error::Wire)?) as usize;
        if sequence == u64::MAX
            || !(16..=MAX_PLAINTEXT + 16).contains(&length)
            || bytes.len() != HEADER + length + 16
        {
            return Err(Error::Wire);
        }
        Ok(Self {
            circuit: bytes[10..42].try_into().map_err(|_| Error::Wire)?,
            reply: bytes[9] == 1,
            sequence,
            body: &bytes[HEADER..HEADER + length],
            tag: bytes[HEADER + length..]
                .try_into()
                .map_err(|_| Error::Wire)?,
        })
    }
}
