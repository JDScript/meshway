use super::{
    Channel, Incoming, Inner, Outbound, StreamRole, lock,
    wire::{HOP_ACK, HOP_CONFIRM, hop_prologue},
};
use crate::{
    DuplexStream, Error, OutgoingCandidate, Result, TransportKind,
    crypto::{Cipher, read_handshake, write_handshake},
    framing::{read_record, write_record},
};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, mpsc, watch},
};

pub(super) struct Established<S> {
    stream: S,
    cipher: Cipher,
    peer: String,
    priority: u16,
}

pub(super) async fn authenticate<S: DuplexStream>(
    inner: &Inner,
    mut stream: S,
    role: StreamRole,
    transport: TransportKind,
    priority: u16,
) -> Result<Established<S>> {
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return Err(Error::Closed);
    }
    let timeout = Duration::from_millis(inner.config.limits.handshake_timeout_ms);
    let initiator = matches!(role, StreamRole::Dialer { .. });
    let handshake = async {
        let (mut state, expected) = match &role {
            StreamRole::Dialer { peer } => {
                inner.authorize_transport(peer, transport)?;
                let key = inner.keys.get(peer).ok_or(Error::Authentication)?;
                (
                    inner
                        .identity
                        .handshake(Some(key), &hop_prologue(transport))?,
                    Some(peer.as_str()),
                )
            }
            StreamRole::Acceptor => (
                inner.identity.handshake(None, &hop_prologue(transport))?,
                None,
            ),
        };
        let setup = [1, (priority >> 8) as u8, priority as u8];
        let negotiated = match role {
            StreamRole::Dialer { .. } => {
                let first = write_handshake(&mut state, &setup)?;
                write_record(&mut stream, &first).await?;
                let response = read_record(&mut stream).await?;
                if read_handshake(&mut state, &response)? != setup {
                    return Err(Error::Protocol);
                }
                priority
            }
            StreamRole::Acceptor => {
                let first = read_record(&mut stream).await?;
                let setup = read_handshake(&mut state, &first)?;
                if setup.len() != 3 || setup[0] != 1 {
                    return Err(Error::Protocol);
                }
                // Authenticate before replying or installing any channel.
                let peer = inner.identify(state.get_remote_static())?;
                inner.authorize_transport(&peer, transport)?;
                let response = write_handshake(&mut state, &setup)?;
                write_record(&mut stream, &response).await?;
                u16::from_be_bytes([setup[1], setup[2]])
            }
        };
        let peer = inner.identify(state.get_remote_static())?;
        if expected.is_some_and(|expected| expected != peer) {
            return Err(Error::Authentication);
        }
        // IK's first flight is replayable. Do not publish a channel until the
        // initiator demonstrates possession of this fresh responder's transport key.
        // These records consume the same cipher nonces/window used by later data.
        let mut cipher = Cipher::finish(state)?;
        if initiator {
            write_record(&mut stream, &cipher.seal(HOP_CONFIRM)?).await?;
            let ack = read_record(&mut stream).await?;
            if cipher.open(&ack)? != HOP_ACK {
                return Err(Error::Authentication);
            }
        } else {
            let confirmation = read_record(&mut stream).await?;
            if cipher.open(&confirmation)? != HOP_CONFIRM {
                return Err(Error::Authentication);
            }
            write_record(&mut stream, &cipher.seal(HOP_ACK)?).await?;
        }
        Ok(Established {
            stream,
            cipher,
            peer,
            priority: negotiated,
        })
    };
    tokio::select! {
        _ = shutdown.changed() => Err(Error::Closed),
        result = tokio::time::timeout(timeout, handshake) => result.map_err(|_| Error::Timeout)?,
    }
}

pub(super) fn install<S: DuplexStream + 'static>(
    inner: Arc<Inner>,
    established: Established<S>,
    transport: TransportKind,
    permit: OwnedSemaphorePermit,
) -> Result<()> {
    let (tx, rx) = mpsc::channel(inner.config.limits.queue_capacity);
    let (close, closed) = watch::channel(false);
    inner.register(
        established.peer.clone(),
        Channel {
            token: established.cipher.id,
            priority: established.priority,
            transport,
            tx,
            close,
        },
    )?;
    inner.clone().spawn(async move {
        let peer = established.peer.clone();
        let token = established.cipher.id;
        let _permit = permit;
        if run(&inner, established, transport, rx, closed)
            .await
            .is_err()
        {
            inner
                .counters(transport)
                .dropped
                .fetch_add(1, Ordering::Relaxed);
        }
        inner.unregister(&peer, &token);
    });
    Ok(())
}

async fn run<S: DuplexStream>(
    inner: &Inner,
    established: Established<S>,
    transport: TransportKind,
    mut rx: mpsc::Receiver<Outbound>,
    mut closed: watch::Receiver<bool>,
) -> Result<()> {
    let Established {
        stream,
        cipher,
        peer,
        ..
    } = established;
    let token = cipher.id;
    let cipher = Mutex::new(cipher);
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() || *closed.borrow() {
        return Ok(());
    }
    let reads = async {
        loop {
            let record = read_record(&mut reader).await?;
            if !inner.active(&peer, &token) {
                return Err(Error::Closed);
            }
            let body = lock(&cipher).open(&record)?;
            inner.deliver(Incoming {
                peer: peer.clone(),
                token,
                transport,
                body,
            });
        }
    };
    let writes = async {
        while let Some(outbound) = rx.recv().await {
            if outbound.token != token {
                return Err(Error::Protocol);
            }
            for frame in outbound.batch.into_frames() {
                let record = lock(&cipher).seal(&frame)?;
                write_record(&mut writer, &record).await?;
                inner
                    .counters(transport)
                    .sent
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(())
    };
    // Entire read/write loops remain pinned throughout selection. An outbound write
    // cannot cancel a partially-read length prefix or payload.
    tokio::select! {
        result = reads => result,
        result = writes => result,
        _ = shutdown.changed() => Ok(()),
        _ = closed.changed() => Ok(()),
    }
}

pub(super) async fn listen(inner: Arc<Inner>, listener: TcpListener) {
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return;
    }
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let Ok((socket, _)) = accepted else { break };
                let Ok(permit) = inner.semaphore.clone().try_acquire_owned() else {
                    inner.counters(TransportKind::NativeTcp).dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let inner = inner.clone();
                inner.clone().spawn(async move {
                    match authenticate(&inner, socket, StreamRole::Acceptor, TransportKind::NativeTcp, 0).await {
                        Ok(established) => { let _ = install(inner, established, TransportKind::NativeTcp, permit); }
                        Err(_) => { inner.counters(TransportKind::NativeTcp).dropped.fetch_add(1, Ordering::Relaxed); }
                    }
                });
            }
        }
    }
}

pub(super) async fn maintain(inner: Arc<Inner>, candidate: OutgoingCandidate) {
    let mut shutdown = inner.shutdown.subscribe();
    let timeout = Duration::from_millis(inner.config.limits.handshake_timeout_ms);
    let delay = Duration::from_millis(inner.config.limits.reconnect_delay_ms);
    loop {
        if *shutdown.borrow() {
            break;
        }
        let need = lock(&inner.peers)
            .get(&candidate.peer)
            .is_none_or(|peer| peer.priority > candidate.priority);
        if need && let Ok(permit) = inner.semaphore.clone().try_acquire_owned() {
            let attempt = async {
                let socket = tokio::time::timeout(timeout, TcpStream::connect(&candidate.endpoint))
                    .await
                    .map_err(|_| Error::Timeout)??;
                let established = authenticate(
                    &inner,
                    socket,
                    StreamRole::Dialer {
                        peer: candidate.peer.clone(),
                    },
                    TransportKind::NativeTcp,
                    candidate.priority,
                )
                .await?;
                install(inner.clone(), established, TransportKind::NativeTcp, permit)
            };
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = attempt => (),
            }
        }
        tokio::select! { _ = shutdown.changed() => break, _ = tokio::time::sleep(delay) => () }
    }
}
