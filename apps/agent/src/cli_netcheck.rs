use clap::{Args as ClapArgs, CommandFactory};
use meshway_agent::netcheck::{Config, Report, measure, parse_endpoint};
use std::net::SocketAddr;

#[derive(ClapArgs)]
pub struct Args {
    /// Explicit numeric UDP observer endpoint; repeat 1..4 times. No DNS or public defaults.
    #[arg(long, required = true, value_parser = parse_endpoint)]
    stun_server: Vec<SocketAddr>,
    /// Independent temporary socket. IPv6 requires explicit [::]:0 or [::1]:0.
    #[arg(long, default_value = "0.0.0.0:0", value_parser = parse_endpoint)]
    bind: SocketAddr,
    /// One absolute timeout per probe, including send and receive.
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(250..=3000))]
    timeout_ms: u64,
    #[arg(long)]
    json: bool,
}

pub async fn execute(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let config =
        Config::new(args.stun_server, args.bind, args.timeout_ms).unwrap_or_else(|error| {
            crate::Cli::command()
                .error(clap::error::ErrorKind::ValueValidation, error)
                .exit()
        });
    let report = measure(config).await?;
    if args.json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        print_human(&report);
    }
    if !report.udp_observed {
        return Err("netcheck: no valid UDP STUN mapping observed; see sample outcomes (this does not prove all UDP is blocked)".into());
    }
    Ok(())
}

fn print_human(report: &Report) {
    println!(
        "Independent UDP probe: {} ({:?})",
        report.bound_socket, report.address_family
    );
    for sample in &report.samples {
        if let (Some(endpoint), Some(rtt)) = (&sample.mapped_endpoint, sample.rtt_ms) {
            println!("  {}: mapped {} ({} ms)", sample.server, endpoint, rtt);
        } else {
            println!(
                "  {}: {:?} ({} ms)",
                sample.server, sample.outcome, sample.elapsed_ms
            );
        }
    }
    let known = |value| match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    };
    println!("UDP observed: {}", report.udp_observed);
    println!(
        "Mapping stable during check: {}",
        known(report.mapping_stable_during_check)
    );
    println!(
        "Mapping varies by tested destination IPs: {}",
        known(report.mapping_varies_by_destination)
    );
    println!(
        "Observation window: 30s from {}; expires {}. Temporary socket is now closed; mappings are not usable carrier candidates.",
        report.measured_at, report.valid_until
    );
    println!(
        "STUN does not authenticate a Meshway peer or establish hole punching. IPv6 underlay observations do not imply IPv6 mesh support."
    );
}
