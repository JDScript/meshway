use crate::{Error, Result};
use std::fmt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_SIZE: usize = 16_384;
pub const MAX_BATCH_FRAMES: usize = 16;
pub const MAX_BATCH_BYTES: usize = 65_536;

pub trait DuplexStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> DuplexStream for T {}
pub type BoxedStream = Box<dyn DuplexStream>;

/// Owned, bounded carrier work. No packet-sized boxed future is required.
pub struct Batch {
    frames: Vec<Vec<u8>>,
}

impl Batch {
    pub fn new(frames: Vec<Vec<u8>>) -> Result<Self> {
        if frames.is_empty()
            || frames.len() > MAX_BATCH_FRAMES
            || frames
                .iter()
                .any(|v| v.is_empty() || v.len() > MAX_FRAME_SIZE)
            || frames.iter().map(Vec::len).sum::<usize>() > MAX_BATCH_BYTES
        {
            return Err(Error::Oversize);
        }
        Ok(Self { frames })
    }
    pub fn into_frames(self) -> Vec<Vec<u8>> {
        self.frames
    }
    pub fn len(&self) -> usize {
        self.frames.len()
    }
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl fmt::Debug for Batch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Batch")
            .field("frames", &self.frames.len())
            .finish_non_exhaustive()
    }
}

pub(crate) async fn read_record<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Vec<u8>> {
    let size = stream.read_u32().await? as usize;
    if size == 0 || size > MAX_FRAME_SIZE {
        return Err(Error::Oversize);
    }
    let mut frame = vec![0; size];
    stream.read_exact(&mut frame).await?;
    Ok(frame)
}

pub(crate) async fn write_record<W: AsyncWrite + Unpin>(
    stream: &mut W,
    frame: &[u8],
) -> Result<()> {
    if frame.is_empty() || frame.len() > MAX_FRAME_SIZE {
        return Err(Error::Oversize);
    }
    stream.write_u32(frame.len() as u32).await?;
    stream.write_all(frame).await?;
    stream.flush().await?;
    Ok(())
}
