use super::{
    Channel, DropReason, Failure, Incoming, Inner, Outbound, StageResult, StreamContext,
    StreamRole, lock,
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
    initiator: bool,
    context: StreamContext,
}

pub(super) async fn authenticate_with_context<S: DuplexStream>(
    inner: &Inner,
    mut stream: S,
    role: StreamRole,
    transport: TransportKind,
    priority: u16,
    context: StreamContext,
) -> Result<Established<S>> {
    context.validate(&inner.config, &role, transport)?;
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
                context.authorize(&peer)?;
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
            initiator,
            context,
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
            initiator: established.initiator,
            context: Arc::new(established.context.clone()),
            probe_budget: Default::default(),
        },
    )?;
    inner.clone().spawn(async move {
        let peer = established.peer.clone();
        let token = established.cipher.id;
        let _permit = permit;
        if let Err(failure) = run(&inner, established, transport, rx, closed).await {
            inner.counters(transport).drop(failure.reason);
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
) -> StageResult<()> {
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
                return Err(Failure::new(Error::Closed, DropReason::StaleChannel));
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
                return Err(Failure::new(Error::Protocol, DropReason::StaleChannel));
            }
            for frame in outbound.batch.into_frames() {
                let record = lock(&cipher).seal(&frame)?;
                write_record(&mut writer, &record).await?;
                if frame.first() != Some(&super::neighbor::NAMESPACE) {
                    inner
                        .counters(transport)
                        .sent
                        .fetch_add(1, Ordering::Relaxed);
                }
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

pub(super) async fn listen(inner: Arc<Inner>, listener: TcpListener, context: StreamContext) {
    let mut shutdown = inner.shutdown.subscribe();
    if *shutdown.borrow() {
        return;
    }
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let Ok((socket, remote_addr)) = accepted else { break };
                let Ok(permit) = inner.semaphore.clone().try_acquire_owned() else {
                    inner.counters(TransportKind::NativeTcp).drop(DropReason::CapacityOther);
                    continue;
                };
                let inner = inner.clone();
                let context = StreamContext { remote_addr: Some(remote_addr), ..context.clone() };
                inner.clone().spawn(async move {
                    if let Err(error) = configure_socket(&socket, inner.config.limits.native_tcp_nodelay) {
                        inner.counters(TransportKind::NativeTcp).drop(error.into());
                        return;
                    }
                    match authenticate_with_context(&inner, socket, StreamRole::Acceptor, TransportKind::NativeTcp, 0, context).await {
                        Ok(established) => { let _ = install(inner, established, TransportKind::NativeTcp, permit); }
                        Err(error) => { inner.counters(TransportKind::NativeTcp).drop(error.into()); }
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
                configure_socket(&socket, inner.config.limits.native_tcp_nodelay)?;
                let context = StreamContext {
                    candidate_id: candidate.id.clone(),
                    remote_addr: Some(socket.peer_addr()?),
                    ..Default::default()
                };
                let established = authenticate_with_context(
                    &inner,
                    socket,
                    StreamRole::Dialer {
                        peer: candidate.peer.clone(),
                    },
                    TransportKind::NativeTcp,
                    candidate.priority,
                    context,
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

pub(super) fn configure_socket(socket: &TcpStream, nodelay: bool) -> Result<()> {
    socket.set_nodelay(nodelay)?;
    Ok(())
}
