use crate::Arguments;
use meshway_dataplane::{
    Action, DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketPool,
    secure::{Received, SecureNode},
};
use meshway_logging::profile::{self, Metric};
use meshway_static_authority::{
    Error,
    fixture::{Chain, tcp_ipv4},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    calls: u64,
    cpu_ns: u64,
    wall_ns: u64,
}

impl Timing {
    fn measure<T>(&mut self, action: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
        let cpu = cpu_time()?;
        let wall = Instant::now();
        let output = action()?;
        let wall = nanos(wall.elapsed())?;
        let cpu = cpu_time()?
            .checked_sub(cpu)
            .ok_or("thread CPU clock moved backwards")?;
        self.calls += 1;
        self.cpu_ns += cpu;
        self.wall_ns += wall;
        Ok(output)
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    roles: BTreeMap<String, Timing>,
    /// Exact bytes of each one-way Data frame, in traversal order.
    frame_bytes: BTreeMap<String, Vec<usize>>,
    delivered_packets: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    nodes: u8,
    packet_bytes: u32,
    rounds: u32,
    warmup: u32,
    profiling: bool,
    setup_wall_ns: u64,
    harness_cpu_ns: u64,
    harness_wall_ns: u64,
    packets_per_second: f64,
    empty_timer_cpu_ns_per_call: f64,
    measured: Stats,
}

fn nanos(duration: Duration) -> Result<u64, Error> {
    Ok(duration.as_nanos().try_into()?)
}

fn cpu_time() -> Result<u64, Error> {
    let time = nix::time::clock_gettime(nix::time::ClockId::CLOCK_THREAD_CPUTIME_ID)?;
    u64::try_from(time.tv_sec())?
        .checked_mul(1_000_000_000)
        .and_then(|sec| sec.checked_add(u64::try_from(time.tv_nsec()).ok()?))
        .ok_or_else(|| "invalid thread CPU time".into())
}

struct Circuit {
    chain: Chain,
    nodes: Vec<SecureNode>,
    id: [u8; 32],
    sequence: [u32; 2],
    buffers: PacketPool,
}

impl Circuit {
    fn open(count: usize) -> Result<Self, Error> {
        let chain = Chain::new(count, [Duration::from_secs(600); 3])?;
        let mut nodes = chain
            .path
            .iter()
            .enumerate()
            .map(|(i, id)| {
                Ok(SecureNode::new(
                    chain.authority.identity(*id)?,
                    Default::default(),
                    chain.carriers_for(i)?,
                )?)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut pending = nodes[0].start(chain.demand, chain.path.clone())?;
        let id = pending.circuit();
        let mut established = false;
        for _ in 0..2 * count {
            let next = chain
                .path
                .iter()
                .position(|node| *node == pending.next())
                .ok_or("unknown next hop")?;
            let carrier = pending.carrier().clone();
            let received = pending.release(|bytes| nodes[next].receive(&carrier, bytes))??;
            pending = match received {
                Received::Forward(frame)
                | Received::Established {
                    response: Some(frame),
                    ..
                } => frame,
                Received::Established {
                    circuit,
                    response: None,
                } if circuit == id && next == 0 => {
                    established = true;
                    break;
                }
                _ => return Err("unexpected handshake result".into()),
            };
        }
        if !established {
            return Err("handshake did not complete".into());
        }
        let mut circuit = Self {
            chain,
            nodes,
            id,
            sequence: [101, 201],
            buffers: PacketPool::new(
                4,
                meshway_dataplane::secure::MAX_PLAINTEXT,
                DEFAULT_HEADROOM,
                DEFAULT_TAILROOM,
            )?,
        };
        let mut setup = Stats::default();
        circuit.packet(false, 100, 0, 2, 52, &mut setup)?;
        circuit.packet(true, 200, 101, 0x12, 52, &mut setup)?;
        circuit.packet(false, 101, 201, 0x10, 52, &mut setup)?;
        Ok(circuit)
    }

    fn round(&mut self, length: usize, stats: &mut Stats) -> Result<(), Error> {
        let payload = u32::try_from(length - 52)?;
        for reply in [false, true] {
            let side = usize::from(reply);
            self.packet(
                reply,
                self.sequence[side],
                self.sequence[1 - side],
                0x10,
                length,
                stats,
            )?;
            self.sequence[side] = self.sequence[side].wrapping_add(payload);
        }
        Ok(())
    }

    fn packet(
        &mut self,
        reply: bool,
        sequence: u32,
        acknowledgement: u32,
        flags: u8,
        length: usize,
        stats: &mut Stats,
    ) -> Result<(), Error> {
        let last = self.nodes.len() - 1;
        let (source, destination) = if reply { (last, 0) } else { (0, last) };
        // Synthetic OS packet construction is outside role timers, but included
        // in harness throughput. Packet parse/owned ingress copy are measured.
        let bytes = tcp_ipv4(
            self.chain.addresses[source],
            self.chain.addresses[destination],
            reply,
            sequence,
            acknowledgement,
            flags,
            length,
        )?;
        let role = if reply { "B.send" } else { "A.send" };
        let mut pending = stats.roles.entry(role.into()).or_default().measure(|| {
            static PROFILE: Metric = Metric::new("l0.send");
            let _profile = PROFILE.enter();
            let mut buffer = self.buffers.acquire(bytes.len())?;
            buffer.copy_from_slice(&bytes);
            Ok(self.nodes[source].send(self.id, Packet::parse(buffer)?)?)
        })?;
        let mut sizes = Vec::with_capacity(last);
        for cut in 1..=last {
            let next = if reply { last - cut } else { cut };
            if pending.next() != self.chain.path[next] {
                return Err("wrong next hop".into());
            }
            let carrier = pending.carrier().clone();
            let role = if next == destination {
                if reply {
                    "A.receive".into()
                } else {
                    "B.receive".into()
                }
            } else {
                format!("R{next}.forward")
            };
            let received = stats.roles.entry(role).or_default().measure(|| {
                static PROFILE: Metric = Metric::new("l0.receive");
                let _profile = PROFILE.enter();
                // Actual synchronous receiver is the release sink. Incoming
                // release checks are charged to the receiving role, never skipped.
                let received = pending.release(|wire| {
                    sizes.push(wire.len());
                    self.nodes[next].receive(&carrier, wire)
                })??;
                match received {
                    Received::Packet(action) => {
                        action.release(|action| -> Result<(), Error> {
                            let Action::Deliver { packet, route } = action else {
                                return Err("not delivery".into());
                            };
                            if next != destination
                                || packet.as_bytes() != bytes
                                || route.original_path != self.chain.path
                            {
                                return Err("delivered bytes/path differ".into());
                            }
                            Ok(())
                        })??;
                        Ok(None)
                    }
                    Received::Forward(frame) if next != destination => Ok(Some(frame)),
                    _ => Err("unexpected Data result".into()),
                }
            })?;
            match received {
                Some(frame) => pending = frame,
                None => {
                    if cut != last {
                        return Err("early delivery".into());
                    }
                    stats.delivered_packets += 1;
                    // v5 has a constant 86-byte overhead. Both
                    // directions must agree with every cut of the byte ledger.
                    for size in &sizes {
                        let expected = length + 86;
                        if *size != expected {
                            return Err("wire size differs from v5 ledger".into());
                        }
                    }
                    let direction = if reply { "reply" } else { "forward" };
                    if let Some(previous) =
                        stats.frame_bytes.insert(direction.into(), sizes.clone())
                        && previous != sizes
                    {
                        return Err("frame length changed".into());
                    }
                    return Ok(());
                }
            }
        }
        Err("Data did not reach terminal".into())
    }
}

pub fn run(args: &Arguments) -> Result<Report, Error> {
    // report() is intentionally paced by the shared logging owner. Capture a
    // pre/post cumulative pair, excluding setup/warmup by subtraction in script.
    profile::report();
    let setup = Instant::now();
    let mut circuit = Circuit::open(usize::from(args.nodes))?;
    for _ in 0..args.warmup {
        circuit.round(args.packet_bytes as usize, &mut Stats::default())?;
    }
    let setup_wall_ns = nanos(setup.elapsed())?;
    let mut calibration = Timing::default();
    for _ in 0..1000 {
        calibration.measure(|| {
            std::hint::black_box(());
            Ok(())
        })?;
    }
    if profile::enabled() {
        std::thread::sleep(Duration::from_millis(5010));
        profile::report();
    }
    let mut measured = Stats::default();
    let cpu = cpu_time()?;
    let wall = Instant::now();
    for _ in 0..args.rounds {
        circuit.round(args.packet_bytes as usize, &mut measured)?;
    }
    let harness_wall_ns = nanos(wall.elapsed())?;
    let harness_cpu_ns = cpu_time()?
        .checked_sub(cpu)
        .ok_or("thread CPU clock moved backwards")?;
    if measured.delivered_packets != u64::from(args.rounds) * 2 {
        return Err("delivery count differs".into());
    }
    if profile::enabled() {
        std::thread::sleep(Duration::from_millis(5010));
        profile::report();
    }
    Ok(Report {
        nodes: args.nodes,
        packet_bytes: args.packet_bytes,
        rounds: args.rounds,
        warmup: args.warmup,
        profiling: profile::enabled(),
        setup_wall_ns,
        harness_cpu_ns,
        harness_wall_ns,
        packets_per_second: measured.delivered_packets as f64 * 1e9 / harness_wall_ns as f64,
        empty_timer_cpu_ns_per_call: calibration.cpu_ns as f64 / calibration.calls as f64,
        measured,
    })
}
