use crate::{Arguments, Error};
use meshway_transport_udp::UdpTransport;
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(super) struct RawUdp {
    client: UdpTransport,
    destination: SocketAddr,
    peer: Option<JoinHandle<Result<(), Error>>>,
    stop: CancellationToken,
    payload_bytes: usize,
}

impl RawUdp {
    pub(super) fn new(
        client: UdpTransport,
        server: UdpTransport,
        args: &Arguments,
    ) -> Result<Self, Error> {
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let destination = server.local_addr()?;
        let source = client.local_addr()?;
        let payload_bytes = args.payload_bytes as usize;
        let batch_size = args.batch_size;
        let counts = [args.warmup, args.messages];
        let peer = tokio::spawn(async move {
            tokio::select! { biased;
                _ = cancelled.cancelled() => Ok(()),
                result = async {
                    for mut remaining in counts {
                        while remaining > 0 {
                            let count = remaining.min(batch_size);
                            let mut batch = Vec::with_capacity(count as usize);
                            for _ in 0..count {
                                let mut packet = vec![0; payload_bytes];
                                let received = server.recv_from(&mut packet).await?;
                                if received.source != source || received.truncated || received.copied_len != payload_bytes {
                                    return Err("invalid raw UDP request".into());
                                }
                                batch.push(packet);
                            }
                            for packet in batch {
                                if server.send_to(&packet, source).await? != payload_bytes {
                                    return Err("partial raw UDP echo".into());
                                }
                            }
                            remaining -= count;
                        }
                    }
                    Ok::<_, Error>(())
                } => result,
            }
        });
        Ok(Self {
            client,
            destination,
            peer: Some(peer),
            stop,
            payload_bytes,
        })
    }

    pub(super) async fn exchange(
        &mut self,
        packets: Vec<Vec<u8>>,
    ) -> Result<Vec<(Vec<u8>, Duration)>, Error> {
        let start = Instant::now();
        let sent = async {
            for packet in &packets {
                if self.client.send_to(packet, self.destination).await? != packet.len() {
                    return Err("partial raw UDP send".into());
                }
            }
            Ok::<_, Error>(())
        };
        let received = async {
            let mut echoed = Vec::with_capacity(packets.len());
            for _ in &packets {
                let mut packet = vec![0; self.payload_bytes];
                let received = self.client.recv_from(&mut packet).await?;
                if received.source != self.destination
                    || received.truncated
                    || received.copied_len != self.payload_bytes
                {
                    return Err("invalid raw UDP echo".into());
                }
                echoed.push((packet, start.elapsed()));
            }
            Ok::<_, Error>(echoed)
        };
        let (_, echoed) = tokio::try_join!(sent, received)?;
        Ok(echoed)
    }

    pub(super) async fn close(&mut self) -> Result<(), Error> {
        self.stop.cancel();
        if let Some(peer) = self.peer.take() {
            peer.await??;
        }
        Ok(())
    }
}

impl Drop for RawUdp {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(peer) = &self.peer {
            peer.abort();
        }
    }
}
