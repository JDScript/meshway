use meshway_dataplane::exit::ProjectionScope;
use meshway_underlay::{ExitSendGuard, Invalidated};
use tokio_util::sync::CancellationToken;

pub(super) struct PacketGuard {
    pub scope: ProjectionScope,
    pub stop: CancellationToken,
}
impl ExitSendGuard for PacketGuard {
    fn check(&self) -> Result<(), Invalidated> {
        if self.stop.is_cancelled() {
            return Err(Invalidated);
        }
        self.scope.ensure_current().map_err(|_| Invalidated)
    }
}

pub(super) fn tuple(
    tuple: meshway_dataplane::exit::ExitTuple,
) -> Option<meshway_underlay::OriginalTuple> {
    use meshway_configuration::execution::BusinessProtocol;
    use meshway_underlay::ExitTransport;
    let transport = match tuple.protocol {
        BusinessProtocol::Tcp => ExitTransport::Tcp {
            source_port: tuple.source_port?,
            destination_port: tuple.destination_port?,
        },
        BusinessProtocol::Udp => ExitTransport::Udp {
            source_port: tuple.source_port?,
            destination_port: tuple.destination_port?,
        },
        BusinessProtocol::IcmpEcho => {
            let (identifier, sequence) = tuple.echo?;
            ExitTransport::IcmpEcho {
                identifier,
                sequence,
            }
        }
    };
    Some(meshway_underlay::OriginalTuple {
        source: tuple.source,
        destination: tuple.destination,
        transport,
    })
}
