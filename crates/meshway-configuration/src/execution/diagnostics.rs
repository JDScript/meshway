use crate::object::tagged;
use meshway_transport::spec::Port;

tagged! {
/// Exact service scope checked by Access and routing for a diagnostic operation.
pub enum ObservationServiceScope {
    /// Diagnose a path authorized for a specific TCP service.
    Tcp {
        /// TCP destination port used for service authorization.
        port: Port
    },
    /// Diagnose a path authorized for a specific UDP service.
    Udp {
        /// UDP destination port used for service authorization.
        port: Port
    },
    /// Diagnose a path authorized for ICMP echo without a transport port.
    IcmpEcho,
} }
