use crate::{Error, NodeOptions};
use futures_util::FutureExt;
use meshway_logging::profile::{self, Metric};
use meshway_transport::{Frame, Limits, Receiver, SendContext, SendOutcome, Sender};
use meshway_transport_spec::SocketOptions;
use meshway_transport_udp::UdpTransport;
use meshway_tun::{Options, Tun};
use std::{collections::VecDeque, io::Write, net::SocketAddr, time::Duration};
use tokio_util::sync::CancellationToken;

static TUN_READ: Metric = Metric::new("plain.tun.read");
static UDP_SEND: Metric = Metric::new("plain.udp.send");
static UDP_RECV: Metric = Metric::new("plain.udp.recv");
static TUN_WRITE: Metric = Metric::new("plain.tun.write");

pub async fn run(options: &NodeOptions, stop: CancellationToken) -> Result<(), Error> {
    options.validate()?;
    let tun = Tun::create(Options::new(options.mtu)?.with_ipv4(options.role.mesh_address(), 30)?)?;
    let socket = tokio::net::UdpSocket::bind(options.bind).await?;
    let endpoint = UdpTransport::adopt(socket, &SocketOptions::default())?;
    // Fixed budget across event-batch variants; a single admitted operation.
    let limits = Limits::new(options.mtu, 1024, options.mtu * 1024, 1, 1024)?;
    let (mut sender, mut receiver) = endpoint.into_session(limits).into_split();
    println!(
        "{}",
        serde_json::json!({"ready": true, "stage": options.stage,
        "role": options.role, "tun": tun.name(), "meshAddress": options.role.mesh_address(),
        "mtu": options.mtu, "batch": options.batch, "kernelBatch": false,
        "profile": profile::enabled()})
    );
    std::io::stdout().flush()?;
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => Ok(()),
        result = async {
            tokio::try_join!(
                outbound(&tun, &mut sender, options.peer, options.batch, &stop),
                inbound(&tun, &mut receiver, options.peer, options.batch),
            )?;
            Ok::<_, Error>(())
        } => result,
        _ = async {
            let mut timer = tokio::time::interval(Duration::from_secs(5));
            loop { timer.tick().await; profile::report(); }
        } => unreachable!(),
    };
    stop.cancel();
    sender.shutdown().await?;
    result
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
                    buffers.push(frame.frame.payload);
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
