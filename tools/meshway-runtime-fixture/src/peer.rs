//! Remote OS TCP acceptor for the production underlay library's ignored tests.
use crate::{
    Cli, Result,
    signals::{Signal, Signals},
};
use std::net::{Ipv4Addr, Ipv6Addr};
use tokio::net::TcpListener;

/// Serves until the supervisor's stop signal, at most `stop_within`: the
/// supervisor signals it after its last kernel test, so the bound covers
/// that whole sequence; it only ends a peer whose supervisor never signals.
pub async fn run(
    cli: &Cli,
    ipv4: Ipv4Addr,
    ipv6: Ipv6Addr,
    stop_within: std::time::Duration,
) -> Result<()> {
    let v4 = TcpListener::bind((ipv4, 42427)).await?;
    let v6 = TcpListener::bind((ipv6, 42427)).await?;
    let signals = Signals::new(&cli.signals);
    signals.write("underlay-peer-ready", &Signal::Ready).await?;
    let stopped = signals.wait_for("underlay-peer-stop", stop_within);
    tokio::pin!(stopped);
    let mut connections = 0_u16;
    loop {
        tokio::select! {
            result = &mut stopped => {
                if !matches!(result?, Signal::Ready) { return Err("invalid underlay peer stop signal".into()); }
                break;
            },
            accepted = v4.accept() => { drop(accepted?); connections += 1; },
            accepted = v6.accept() => { drop(accepted?); connections += 1; },
        }
        if connections > 256 {
            return Err("underlay peer connection bound exceeded".into());
        }
    }
    // connect() can complete just before stop is observed; consume the bounded
    // listener backlogs before recording the accepted-connection evidence.
    for listener in [&v4, &v6] {
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
            {
                Ok(Ok(stream)) => {
                    drop(stream);
                    connections += 1;
                }
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => break,
            }
            if connections > 256 {
                return Err("underlay peer connection bound exceeded".into());
            }
        }
    }
    if connections < 4 {
        return Err("underlay tests did not reach both remote listeners".into());
    }
    tracing::info!(
        phase = "underlay-peer-joined",
        connections,
        "bounded remote OS listeners stopped"
    );
    Ok(())
}
