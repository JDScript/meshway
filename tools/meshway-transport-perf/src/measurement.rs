use crate::{Arguments, Error, Mode, Protocol, connection::Connection};
use hdrhistogram::Histogram;
use serde::Serialize;
use std::time::Instant;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Measurement {
    protocol: Protocol,
    mode: Mode,
    payload_bytes: u32,
    batch_size: u32,
    received_messages: u32,
    elapsed_seconds: f64,
    goodput_mbps: f64,
    messages_per_second: f64,
    rtt_p50_ms: f64,
    rtt_p99_ms: f64,
    mesh_authentication: bool,
    mesh_encryption: bool,
    routing: bool,
    tun: bool,
}

pub(crate) async fn run(
    args: &Arguments,
    connection: &mut Connection,
) -> Result<Measurement, Error> {
    let template = vec![0x5a; args.payload_bytes as usize];
    let mut histogram = Histogram::<u64>::new(3)?;
    let mut sequence = 0u64;
    let mut elapsed = None;
    for (count, measured) in [(args.warmup, false), (args.messages, true)] {
        let start = Instant::now();
        let mut remaining = count;
        while remaining > 0 {
            let size = remaining.min(args.batch_size) as usize;
            let packets: Vec<Vec<u8>> = (sequence..sequence + size as u64)
                .map(|id| {
                    let mut packet = template.clone();
                    packet[..8].copy_from_slice(&id.to_be_bytes());
                    packet
                })
                .collect();
            let echoed = connection.exchange(packets).await?;
            if echoed.len() != size {
                return Err("incomplete echo batch".into());
            }
            let mut received = vec![false; size];
            for (packet, rtt) in echoed {
                if packet.len() != template.len() || packet[8..] != template[8..] {
                    return Err("corrupt echoed payload".into());
                }
                let id = u64::from_be_bytes(packet[..8].try_into()?);
                let offset = id.checked_sub(sequence).ok_or("echo from another batch")?;
                let entry = received
                    .get_mut(usize::try_from(offset)?)
                    .ok_or("echo sequence outside batch")?;
                if *entry {
                    return Err("duplicate echo".into());
                }
                *entry = true;
                if measured {
                    histogram.record(u64::try_from(rtt.as_nanos())?)?;
                }
            }
            sequence += size as u64;
            remaining -= size as u32;
        }
        if measured {
            elapsed = Some(start.elapsed().as_secs_f64());
        }
    }
    let elapsed_seconds = elapsed.ok_or("missing measured interval")?;
    if elapsed_seconds <= 0.0 || histogram.is_empty() {
        return Err("measurement interval too short".into());
    }
    Ok(Measurement {
        protocol: args.protocol,
        mode: args.mode,
        payload_bytes: args.payload_bytes,
        batch_size: args.batch_size,
        received_messages: args.messages,
        elapsed_seconds,
        goodput_mbps: f64::from(args.payload_bytes) * f64::from(args.messages) * 8.0
            / elapsed_seconds
            / 1_000_000.0,
        messages_per_second: f64::from(args.messages) / elapsed_seconds,
        rtt_p50_ms: histogram.value_at_quantile(0.5) as f64 / 1_000_000.0,
        rtt_p99_ms: histogram.value_at_quantile(0.99) as f64 / 1_000_000.0,
        mesh_authentication: false,
        mesh_encryption: false,
        routing: false,
        tun: false,
    })
}
