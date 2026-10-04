use crate::MAX_MESSAGE_BYTES;
use serde::{Serialize, de::DeserializeOwned};
use std::io::Write;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("local connection failed or was truncated")]
    Io,
    #[error("local message exceeds its size limit")]
    Limit,
    #[error("invalid local control message")]
    Invalid,
}
pub async fn read<T: DeserializeOwned>(
    input: &mut (impl AsyncRead + Unpin),
) -> Result<T, CodecError> {
    let length = input.read_u32().await.map_err(|_| CodecError::Io)? as usize;
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(CodecError::Limit);
    }
    let mut bytes = vec![0; length];
    input
        .read_exact(&mut bytes)
        .await
        .map_err(|_| CodecError::Io)?;
    let mut decoder = serde_json::Deserializer::from_slice(&bytes);
    let message =
        T::deserialize(meshway_serde::MapOnly(&mut decoder)).map_err(|_| CodecError::Invalid)?;
    decoder.end().map_err(|_| CodecError::Invalid)?;
    Ok(message)
}
struct Buffer(Vec<u8>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_MESSAGE_BYTES - self.0.len() {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub async fn write<T: Serialize>(
    output: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> Result<(), CodecError> {
    let mut buffer = Buffer(Vec::new());
    serde_json::to_writer(&mut buffer, value).map_err(|_| CodecError::Limit)?;
    output
        .write_u32(buffer.0.len() as u32)
        .await
        .map_err(|_| CodecError::Io)?;
    output
        .write_all(&buffer.0)
        .await
        .map_err(|_| CodecError::Io)?;
    output.flush().await.map_err(|_| CodecError::Io)
}
