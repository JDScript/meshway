use crate::{Error, NodeOptions};
use futures_util::FutureExt;
use meshway_logging::profile::{self, Metric};
use meshway_transport::spec::SocketOptions;
use meshway_transport::udp::UdpTransport;
use meshway_transport::{
    BatchOutcome, ChannelScope, Frame, Limits, Receiver, ScopedFrame, SendContext, SendOutcome,
    Sender,
};
use meshway_tun::{Options, PacketBatch, Tun};
use std::{collections::VecDeque, io::Write, net::SocketAddr, time::Duration};
use tokio_util::sync::CancellationToken;

static TUN_READ: Metric = Metric::new("plain.tun.read");
static UDP_SEND: Metric = Metric::new("plain.udp.send");
static UDP_RECV: Metric = Metric::new("plain.udp.recv");
static TUN_WRITE: Metric = Metric::new("plain.tun.write");

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IoMode {
    Batch,
    #[default]
    PerPacket,
}

pub async fn run(options: &NodeOptions, stop: CancellationToken) -> Result<(), Error> {
    run_with_io(options, stop, IoMode::PerPacket).await
}

pub async fn run_with_io(
    options: &NodeOptions,
    stop: CancellationToken,
    mode: IoMode,
) -> Result<(), Error> {
    options.validate()?;
    let batch_io = matches!(mode, IoMode::Batch);
    let tun = Tun::create(
        Options::new(options.mtu)?
            .with_batch_io(batch_io)
            .with_ipv4(options.role.mesh_address(), 30)?,
    )?;
    let socket = tokio::net::UdpSocket::bind(options.bind).await?;
    let endpoint = if batch_io {
        UdpTransport::adopt_batch(socket, &SocketOptions::default())?
    } else {
        UdpTransport::adopt(socket, &SocketOptions::default())?
    };
    let report = endpoint.capability_report();
    // Fixed budget across event-batch variants; a single admitted operation.
    let limits = Limits::new(
        options.mtu,
        1024,
        options.mtu * 1024,
        1,
        if batch_io { options.batch } else { 1024 },
    )?;
    let (mut sender, mut receiver) = endpoint.into_session(limits).into_split();
    println!(
        "{}",
        serde_json::json!({"ready": true, "stage": options.stage,
        "role": options.role, "tun": tun.name(), "meshAddress": options.role.mesh_address(),
        "mtu": options.mtu, "batch": options.batch, "io": mode, "kernelBatch": batch_io,
        "capabilities": capabilities(&tun, report()),
        "profile": profile::enabled()})
    );
    std::io::stdout().flush()?;
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => Ok(()),
        result = async {
            if batch_io {
                tokio::try_join!(outbound_batch(&tun, &mut sender, options.peer, options.batch, &stop), inbound_batch(&tun, &mut receiver, options.peer))?;
            } else { tokio::try_join!(
                outbound(&tun, &mut sender, options.peer, options.batch, &stop),
                inbound(&tun, &mut receiver, options.peer, options.batch),
            )?; }
            Ok::<_, Error>(())
        } => result,
        _ = async {
            let mut timer = tokio::time::interval(Duration::from_secs(5));
            loop { timer.tick().await; profile::report(); }
        } => unreachable!(),
    };
    stop.cancel();
    sender.shutdown().await?;
    eprintln!(
        "{}",
        serde_json::json!({"ioCapabilities": capabilities(&tun, report())})
    );
    profile::report_final();
    result
}

fn capabilities(tun: &Tun, udp: meshway_transport::udp::Capabilities) -> serde_json::Value {
    let tun = tun.capabilities();
    serde_json::json!({"tun": {"vnetHeader": tun.vnet_header, "checksum": tun.checksum, "tso4": tun.tso4, "tso6": tun.tso6, "uso4": tun.uso4, "uso6": tun.uso6},
        "udp": {"segment": udp.udp_segment, "gro": udp.udp_gro, "sendmmsg": udp.sendmmsg, "recvmmsg": udp.recvmmsg,
        "gsoSends": udp.gso_sends, "groReceives": udp.gro_receives, "mmsgSends": udp.mmsg_sends, "mmsgReceives": udp.mmsg_receives}})
}

async fn outbound_batch(
    tun: &Tun,
    sender: &mut Sender,
    peer: SocketAddr,
    limit: usize,
    stop: &CancellationToken,
) -> Result<(), Error> {
    let scope = ChannelScope::new();
    let context = SendContext {
        cancellation: stop.clone(),
        ..SendContext::default()
    };
    let mut batch = PacketBatch::default();
    loop {
        TUN_READ.poll(tun.recv_batch(&mut batch)).await?;
        let mut frames: VecDeque<_> = batch
            .take()
            .into_iter()
            .map(|payload| ScopedFrame {
                frame: Frame::new(payload, Some(peer)),
                scope: scope.clone(),
                context: context.clone(),
            })
            .collect();
        while !frames.is_empty() {
            let suffix = if frames.len() > limit {
                frames.split_off(limit)
            } else {
                VecDeque::new()
            };
            let admission = sender.submit_batch(&mut frames, context.clone());
            let completed = UDP_SEND
                .poll(
                    admission
                        .completion
                        .ok_or("batch admission rejected")?
                        .wait(),
                )
                .await?;
            if !matches!(completed.outcome(), BatchOutcome::LocalComplete) {
                return Err("plain UDP batch did not complete".into());
            }
            frames.extend(suffix);
            if !frames.is_empty() {
                tokio::task::yield_now().await;
            }
        }
        tokio::task::yield_now().await;
    }
}

async fn inbound_batch(tun: &Tun, receiver: &mut Receiver, peer: SocketAddr) -> Result<(), Error> {
    loop {
        let frames = UDP_RECV.poll(receiver.recv_batch()).await?;
        if frames.iter().any(|frame| frame.peer != Some(peer)) {
            return Err("unexpected plain UDP source".into());
        }
        let packets: Vec<_> = frames
            .iter()
            .map(|frame| frame.payload.as_slice())
            .collect();
        let mut sent = 0;
        while sent < packets.len() {
            sent += TUN_WRITE.poll(tun.send_batch(&packets[sent..])).await?;
        }
        tokio::task::yield_now().await;
    }
}

async fn outbound(
    tun: &Tun,
    sender: &mut Sender,
    peer: SocketAddr,
    batch: usize,
    stop: &CancellationToken,
) -> Result<(), Error> {
    let mut buffers = Vec::with_capacity(batch);
    for _ in 0..batch {
        buffers.push(vec![0; tun.mtu()]);
    }
    loop {
        let mut frames = VecDeque::with_capacity(batch);
        for index in 0..batch {
            let mut buffer = buffers.pop().ok_or("missing recycled buffer")?;
            buffer.resize(tun.mtu(), 0);
            let read = TUN_READ.poll(tun.recv(&mut buffer));
            let size = if index == 0 {
                Some(read.await)
            } else {
                read.now_or_never()
            };
            match size {
                Some(size) => {
                    buffer.truncate(size?);
                    frames.push_back(Frame::new(buffer, Some(peer)));
                }
                None => {
                    buffers.push(buffer);
                    break;
                }
            }
        }
        UDP_SEND
            .poll(async {
                let count = frames.len();
                let admission = sender.try_send(
                    &mut frames,
                    SendContext {
                        cancellation: stop.clone(),
                        ..SendContext::default()
                    },
                );
                if admission.accepted != count || admission.stop.is_some() {
                    return Err("incomplete plain batch admission".into());
                }
                let completed = admission
                    .completion
                    .ok_or("missing completion")?
                    .wait()
                    .await?;
                for frame in completed.frames {
                    if !matches!(frame.outcome, SendOutcome::LocalComplete) {
                        return Err("plain UDP send did not complete".into());
                    }
                    buffers.push(frame.frame.payload.into_vec());
                }
                Ok::<_, Error>(())
            })
            .await?;
        tokio::task::yield_now().await;
    }
}

async fn inbound(
    tun: &Tun,
    receiver: &mut Receiver,
    peer: SocketAddr,
    batch: usize,
) -> Result<(), Error> {
    loop {
        for index in 0..batch {
            let receive = UDP_RECV.poll(receiver.recv());
            let frame = if index == 0 {
                Some(receive.await)
            } else {
                receive.now_or_never()
            };
            let Some(frame) = frame else {
                break;
            };
            let frame = frame?;
            if frame.peer != Some(peer) {
                return Err("unexpected plain UDP source".into());
            }
            TUN_WRITE.poll(tun.send(&frame.payload)).await?;
        }
        tokio::task::yield_now().await;
    }
}
