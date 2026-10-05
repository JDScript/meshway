use crate::{Arguments, Error};
use std::{
    io::{self, IoSlice},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub(super) struct RawTcp<S> {
    read: BufReader<ReadHalf<S>>,
    write: WriteHalf<S>,
    peer: Option<JoinHandle<Result<(), Error>>>,
    stop: CancellationToken,
    payload_bytes: usize,
    write_buffer: Vec<u8>,
    write_capacity: usize,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send + 'static> RawTcp<S> {
    pub(super) fn new(
        client: S,
        mut server: S,
        args: &Arguments,
        read_capacity: usize,
        write_capacity: usize,
    ) -> Self {
        let (read, write) = tokio::io::split(client);
        let read = BufReader::with_capacity(read_capacity, read);
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let payload_bytes = args.payload_bytes as usize;
        let batch_size = args.batch_size;
        let counts = [args.warmup, args.messages];
        let peer = tokio::spawn(async move {
            tokio::select! { biased;
                _ = cancelled.cancelled() => Ok(()),
                result = async {
                    let mut buffer = vec![0; payload_bytes * batch_size as usize];
                    for mut remaining in counts {
                        while remaining > 0 {
                            let count = remaining.min(batch_size);
                            let bytes = &mut buffer[..count as usize * payload_bytes];
                            server.read_exact(bytes).await?;
                            server.write_all(bytes).await?;
                            server.flush().await?;
                            remaining -= count;
                        }
                    }
                    Ok::<_, Error>(())
                } => result,
            }
        });
        Self {
            read,
            write,
            peer: Some(peer),
            stop,
            payload_bytes,
            write_buffer: Vec::new(),
            write_capacity,
        }
    }

    pub(super) async fn exchange(
        &mut self,
        packets: Vec<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Duration)>, Error> {
        let start = Instant::now();
        let write = &mut self.write;
        let read = &mut self.read;
        let payload_bytes = self.payload_bytes;
        let buffer = &mut self.write_buffer;
        let capacity = self.write_capacity;
        let sent = async {
            if !write.is_write_vectored() {
                // Preserve the same logical batch even for byte-stream
                // protocols without vectored I/O (not one WS message per IP
                // payload). Working storage has the same framing buffer cap.
                buffer.clear();
                if buffer.capacity() < capacity {
                    buffer.reserve_exact(capacity);
                }
                for packet in &packets {
                    if buffer.len() + packet.len() > capacity {
                        write.write_all(buffer).await?;
                        buffer.clear();
                    }
                    buffer.extend_from_slice(packet);
                }
                write.write_all(buffer).await?;
                buffer.clear();
                return write.flush().await;
            }
            let mut slices: Vec<_> = packets.iter().map(|bytes| IoSlice::new(bytes)).collect();
            let mut remaining = slices.as_mut_slice();
            while !remaining.is_empty() {
                let written = write.write_vectored(remaining).await?;
                if written == 0 {
                    return Err(io::Error::from(io::ErrorKind::WriteZero));
                }
                IoSlice::advance_slices(&mut remaining, written);
            }
            write.flush().await
        };
        let received = async {
            let mut echoed = Vec::with_capacity(packets.len());
            for _ in &packets {
                let mut bytes = vec![0; payload_bytes];
                read.read_exact(&mut bytes).await?;
                echoed.push((bytes, start.elapsed()));
            }
            Ok::<_, io::Error>(echoed)
        };
        let (_, echoed) = tokio::try_join!(sent, received)?;
        Ok(echoed)
    }

    pub(super) async fn close(&mut self) -> Result<(), Error> {
        self.stop.cancel();
        super::join_peer(&mut self.peer).await
    }
}

impl<S> Drop for RawTcp<S> {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(peer) = &self.peer {
            peer.abort();
        }
    }
}
