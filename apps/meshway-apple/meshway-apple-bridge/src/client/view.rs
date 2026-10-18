//! FFI presentation values. Optional properties retain the IPC field's meaning:
//! required-nullable properties are null, optional observations are absent.
//! Unknown fields/enums are rejected by IPC before reaching these records.
use meshway_configuration::{NodeAddresses, RouteAction, selection};
use meshway_protocol::ipc;

macro_rules! enumeration {
    ($name:ident, $source:path, $($variant:ident),+ $(,)?) => {
        #[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
        pub enum $name { $($variant),+ }
        impl From<$source> for $name {
            fn from(value: $source) -> Self {
                use $source as Source;
                match value { $(Source::$variant => Self::$variant),+ }
            }
        }
    };
}
enumeration!(
    ClientArtifactProblem,
    ipc::ArtifactProblem,
    Download,
    Integrity,
    Storage,
    Budget
);
enumeration!(
    ClientUnavailableReason,
    meshway_configuration::execution::UnavailableReason,
    ExitUnavailable,
    UnsupportedFamily,
    EgressConflict,
    Unauthorized,
    NoPath,
    RoutingCycle,
    RoutingLimit,
    MissingDependency,
    InactiveNode
);
enumeration!(
    ClientPhase,
    ipc::Phase,
    Down,
    Starting,
    AwaitingApproval,
    Synchronizing,
    ControlReady,
    Running,
    Retrying,
    Blocked,
    Stopping,
    Leaving
);
enumeration!(
    ClientPlane,
    ipc::DataPlane,
    ControlOnly,
    Stopped,
    Preparing,
    Ready,
    Suspended,
    Failed
);
enumeration!(
    ClientSync,
    ipc::SyncState,
    Connected,
    Reconnecting,
    Disconnected
);
enumeration!(
    ClientSyncError,
    ipc::SyncError,
    Network,
    ServerBusy,
    Credentials,
    SignatureInvalid,
    ServerKeyMismatch,
    Withdrawn,
    IdentityRejected,
    Contract,
    Storage
);
enumeration!(
    ClientProblem,
    ipc::Problem,
    Network,
    ServerBusy,
    Credentials,
    EnrollmentExpired,
    EnrollmentWithdrawn,
    EnrollmentCancelled,
    Storage,
    Contract,
    Configuration,
    Clock,
    Worker,
    UnsupportedMode,
    Underlay,
    Tun,
    Dns,
    Transport,
    Apply
);
enumeration!(
    ClientPeerState,
    ipc::PeerState,
    Direct,
    Relay,
    RelayIdle,
    Idle,
    Unreachable,
    Offline
);
enumeration!(ClientIdleRoute, ipc::IdleRoute, Direct, Relay, None);
enumeration!(
    ClientErrorCode,
    ipc::ErrorCode,
    Version,
    Conflict,
    RequestConflict,
    Busy,
    Stopping,
    Storage,
    NotReady,
    Pending,
    Network,
    ServerBusy,
    Credentials,
    Contract,
    Rejected,
    Leaving,
    ProfileNotAllowed,
    ProfileLocked,
    SelectionConflict
);
enumeration!(
    ClientProfileKind,
    selection::ProfileKind,
    Mesh,
    Global,
    Auto,
    Custom
);
enumeration!(
    ClientSelectionUnavailable,
    selection::SelectionUnavailable,
    NotAllowed,
    Locked
);
enumeration!(
    ClientAutoBasis,
    selection::AutoBasis,
    Label,
    NoRuleMatched,
    NoTrustedRegion
);
enumeration!(
    ClientPortMapping,
    ipc::PortMapping,
    Preserved,
    Changed,
    Unknown
);
enumeration!(
    ClientEndpointMapping,
    ipc::EndpointMapping,
    Independent,
    Dependent,
    Unknown
);
enumeration!(
    ClientNetcheckUnavailable,
    ipc::NetcheckUnavailable,
    NotRunning,
    Busy
);
enumeration!(
    ClientPingError,
    ipc::PingError,
    NotRunning,
    Unsupported,
    PermissionDenied,
    TargetNotFound,
    Timeout,
    Busy,
    Stopped,
    Protocol,
    Failed
);
enumeration!(
    ClientEchoOutcome,
    ipc::PingEchoOutcome,
    Reply,
    Unreachable,
    Timeout,
    Failed
);
enumeration!(
    ClientProbeOutcome,
    ipc::ProbeOutcome,
    Pending,
    Success,
    Timeout,
    Failed
);

#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientNode {
    pub hostname: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}
fn node(hostname: String, addresses: NodeAddresses) -> ClientNode {
    ClientNode {
        hostname,
        ipv4: addresses.ipv4.map(|v| v.to_string()),
        ipv6: addresses.ipv6.map(|v| v.to_string()),
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientRtt {
    pub duration_us: u64,
    pub observed_at: std::time::SystemTime,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientMtu {
    pub path: u16,
    pub inner: u16,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientIdlePath {
    pub route: ClientIdleRoute,
    pub via: Vec<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientPeer {
    pub node_id: String,
    pub node: ClientNode,
    pub state: ClientPeerState,
    pub transport: Option<String>,
    pub endpoint: Option<String>,
    pub mtu: Option<ClientMtu>,
    pub via: Vec<String>,
    pub rtt: Option<ClientRtt>,
    pub idle_path: Option<ClientIdlePath>,
}
impl From<ipc::PeerStatus> for ClientPeer {
    fn from(p: ipc::PeerStatus) -> Self {
        Self {
            node_id: p.node_id.to_string(),
            node: node(p.hostname, p.addresses),
            state: p.state.into(),
            transport: p.transport.map(|v| v.as_str().into()),
            endpoint: p.endpoint.map(|v| v.to_string()),
            mtu: p.mtu.map(|v| ClientMtu {
                path: v.path,
                inner: v.inner,
            }),
            via: p.via.iter().map(ToString::to_string).collect(),
            rtt: p.rtt.map(|v| ClientRtt {
                duration_us: v.duration_us.get(),
                observed_at: v.observed_at.into(),
            }),
            idle_path: p.idle_path.map(|v| ClientIdlePath {
                route: v.route.into(),
                via: v.via.iter().map(ToString::to_string).collect(),
            }),
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientIdentity {
    pub node_id: String,
    pub network_id: String,
    pub key_generation: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientInstance {
    pub instance_id: String,
    pub generation: u64,
    pub expires_at: std::time::SystemTime,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientNetmapRef {
    pub revision: u64,
    pub serial: u64,
    pub digest: String,
}
impl From<meshway_model::NetmapRef> for ClientNetmapRef {
    fn from(v: meshway_model::NetmapRef) -> Self {
        Self {
            revision: v.revision.get(),
            serial: v.serial.get(),
            digest: v.digest.to_string(),
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientNetmap {
    pub reference: ClientNetmapRef,
    pub issued_at: std::time::SystemTime,
    pub expires_at: std::time::SystemTime,
    pub missing_artifacts: Vec<String>,
    pub artifact_problem: Option<ClientArtifactProblem>,
    pub routing: Option<ClientSelection>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientNetworkError {
    pub problem: ClientProblem,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientStatus {
    pub process_id: u32,
    pub revision: u64,
    pub desired_up: bool,
    pub phase: ClientPhase,
    pub server: String,
    pub observed_at: std::time::SystemTime,
    pub control_observed_at: Option<std::time::SystemTime>,
    pub enrollment_request_id: Option<String>,
    pub enrollment_url: Option<String>,
    pub identity: Option<ClientIdentity>,
    pub authenticated: bool,
    pub instance: Option<ClientInstance>,
    pub sync: ClientSync,
    pub last_sync_at: Option<std::time::SystemTime>,
    pub sync_error: Option<ClientSyncError>,
    pub netmap: Option<ClientNetmap>,
    pub saved: Option<ClientNetmapRef>,
    pub applied: Option<ClientNetmapRef>,
    pub data_plane: ClientPlane,
    pub network_mode: String,
    pub node: Option<ClientNode>,
    pub peers: Vec<ClientPeer>,
    pub last_error: Option<ClientProblem>,
    pub last_error_detail: Option<String>,
    pub retry_at: Option<std::time::SystemTime>,
    pub network_error: Option<ClientNetworkError>,
    pub last_network_change: Option<std::time::SystemTime>,
}
impl From<ipc::Status> for ClientStatus {
    fn from(v: ipc::Status) -> Self {
        Self {
            process_id: v.process_id,
            revision: v.revision.get(),
            desired_up: v.desired_up,
            phase: v.phase.into(),
            server: v.server,
            observed_at: v.observed_at.into(),
            control_observed_at: v.control_observed_at.map(Into::into),
            enrollment_request_id: v.enrollment_request_id.map(|v| v.to_string()),
            enrollment_url: v.enrollment_url,
            identity: v.identity.map(|v| ClientIdentity {
                node_id: v.node_id.to_string(),
                network_id: v.network_id.to_string(),
                key_generation: v.keys.control.generation.get(),
            }),
            authenticated: v.authenticated,
            instance: v.instance.map(|v| ClientInstance {
                instance_id: v.instance_id.to_string(),
                generation: v.generation.get(),
                expires_at: std::time::UNIX_EPOCH
                    + std::time::Duration::from_millis(v.expires_at_ms),
            }),
            sync: v.sync.into(),
            last_sync_at: v.last_sync_at.map(Into::into),
            sync_error: v.sync_error.map(Into::into),
            netmap: v.netmap.map(|v| ClientNetmap {
                reference: v.netmap.into(),
                issued_at: v.issued_at.into(),
                expires_at: v.expires_at.into(),
                missing_artifacts: v.missing_artifacts,
                artifact_problem: v.artifact_problem.map(Into::into),
                routing: v.routing.map(Into::into),
            }),
            saved: v.saved.map(Into::into),
            applied: v.applied.map(Into::into),
            data_plane: v.data_plane.into(),
            network_mode: match v.network_mode {
                ipc::NetworkMode::Tun => "tun",
                ipc::NetworkMode::ControlOnly => "control-only",
            }
            .into(),
            node: v.node.map(|v| node(v.hostname, v.addresses)),
            peers: v.peers.into_iter().map(Into::into).collect(),
            last_error: v.last_error.map(Into::into),
            last_error_detail: v.last_error_detail,
            retry_at: v.retry_at.map(Into::into),
            network_error: v.network_error.map(|v| ClientNetworkError {
                problem: v.problem.into(),
                detail: v.detail,
            }),
            last_network_change: v.last_network_change.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ClientRouteAction {
    Exit {
        nodes: Vec<String>,
        via: Vec<String>,
    },
    Direct,
    Unmanaged,
}
impl From<RouteAction> for ClientRouteAction {
    fn from(v: RouteAction) -> Self {
        match v {
            RouteAction::Exit { nodes, via } => Self::Exit { nodes, via },
            RouteAction::Direct => Self::Direct,
            RouteAction::Unmanaged => Self::Unmanaged,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientProfile {
    pub name: String,
    pub kind: ClientProfileKind,
    pub rules: u32,
    pub exits: Vec<String>,
    pub fallback: Option<ClientRouteAction>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientAuto {
    pub profile: String,
    pub basis: ClientAutoBasis,
    pub region: Option<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientSelection {
    pub requested_profile: Option<String>,
    pub version: Option<u64>,
    pub applied_profile: String,
    pub requested_unavailable: Option<ClientSelectionUnavailable>,
    pub default_profile: String,
    pub locked: bool,
    pub profiles: Vec<ClientProfile>,
    pub auto: Option<ClientAuto>,
}
impl From<selection::RoutingSelectionState> for ClientSelection {
    fn from(v: selection::RoutingSelectionState) -> Self {
        Self {
            requested_profile: v.requested_profile,
            version: v.version.map(|v| v.get()),
            applied_profile: v.applied_profile,
            requested_unavailable: v.requested_unavailable.map(Into::into),
            default_profile: v.default_profile,
            locked: v.locked,
            profiles: v
                .profiles
                .into_iter()
                .map(|p| ClientProfile {
                    name: p.name,
                    kind: p.kind.into(),
                    rules: p.rules,
                    exits: p.exits.iter().map(ToString::to_string).collect(),
                    fallback: p.fallback.map(Into::into),
                })
                .collect(),
            auto: v.auto.map(|v| ClientAuto {
                profile: v.profile,
                basis: v.basis.into(),
                region: v.region,
            }),
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientMapping {
    pub observer: String,
    pub mapped: String,
    pub rtt_us: Option<u64>,
    pub fresh: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientCheckSocket {
    pub resource: ClientResource,
    pub bound_address: String,
    pub mappings: Vec<ClientMapping>,
    pub port_mapping: ClientPortMapping,
    pub endpoint_mapping: ClientEndpointMapping,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ClientResource {
    Listener { key: String },
    Dial { key: String },
    Coordination { key: String },
}
impl From<meshway_model::state::ResourceRef> for ClientResource {
    fn from(v: meshway_model::state::ResourceRef) -> Self {
        use meshway_model::state::ResourceRef as R;
        match v {
            R::Listener { key } => Self::Listener { key },
            R::Dial { key } => Self::Dial { key },
            R::Coordination { key } => Self::Coordination { key },
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientSizeProbe {
    pub size: u16,
    pub answered: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientObserver {
    pub observer: String,
    pub rtt_us: Option<u64>,
    pub size_probe: Option<ClientSizeProbe>,
    pub path_mtu: Option<u16>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientNetcheck {
    pub observed_at: std::time::SystemTime,
    pub server: String,
    pub server_reachable: Option<bool>,
    pub server_rtt_us: Option<u64>,
    pub unavailable: Option<ClientNetcheckUnavailable>,
    pub ipv4: bool,
    pub ipv6: bool,
    pub udp: bool,
    pub sockets: Vec<ClientCheckSocket>,
    pub observers: Vec<ClientObserver>,
}
impl From<ipc::NetcheckReport> for ClientNetcheck {
    fn from(v: ipc::NetcheckReport) -> Self {
        Self {
            observed_at: v.observed_at.into(),
            server: v.server.origin,
            server_reachable: v.server.reachable,
            server_rtt_us: v.server.rtt_us.map(|v| v.get()),
            unavailable: v.unavailable.map(Into::into),
            ipv4: v.ipv4,
            ipv6: v.ipv6,
            udp: v.udp,
            sockets: v
                .sockets
                .into_iter()
                .map(|v| ClientCheckSocket {
                    resource: v.resource.into(),
                    bound_address: v.bound_address.to_string(),
                    mappings: v
                        .mappings
                        .into_iter()
                        .map(|v| ClientMapping {
                            observer: v.observer.to_string(),
                            mapped: v.mapped.to_string(),
                            rtt_us: v.rtt_us.map(|v| v.get()),
                            fresh: v.fresh,
                        })
                        .collect(),
                    port_mapping: v.port_mapping.into(),
                    endpoint_mapping: v.endpoint_mapping.into(),
                })
                .collect(),
            observers: v
                .observers
                .into_iter()
                .map(|v| ClientObserver {
                    observer: v.observer.to_string(),
                    rtt_us: v.rtt_us.map(|v| v.get()),
                    size_probe: v.size_probe.map(|v| ClientSizeProbe {
                        size: v.size,
                        answered: v.answered,
                    }),
                    path_mtu: v.path_mtu,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum ClientPingAction {
    Mesh,
    Exit { node: String },
    Direct,
    Unmanaged,
    Denied { reason: ClientUnavailableReason },
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ClientResolver {
    MagicDns,
    Exit { node: String },
    Resolvers { keys: Vec<String> },
    System,
    Unavailable,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientResolution {
    pub name: String,
    pub resolver: ClientResolver,
    pub addresses: Vec<String>,
    pub association: Option<ClientAssociation>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientAssociation {
    pub names: Vec<String>,
    pub remaining_seconds: u32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientDecision {
    pub destination: Option<String>,
    pub node: Option<ClientTarget>,
    pub resolution: Option<ClientResolution>,
    pub profile: Option<String>,
    pub rule: Option<ClientRule>,
    pub action: Option<ClientPingAction>,
    pub path: Vec<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientRule {
    pub pointer: String,
    pub remark: String,
    pub rule_set: Option<ClientRuleSetMatch>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientRuleSetMatch {
    pub key: String,
    pub entry: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientTarget {
    pub node_id: String,
    pub node: ClientNode,
}
impl From<meshway_protocol::NodeObservationTargetView> for ClientTarget {
    fn from(v: meshway_protocol::NodeObservationTargetView) -> Self {
        Self {
            node_id: v.node_id.to_string(),
            node: node(v.hostname, v.addresses),
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientEcho {
    pub sequence: u16,
    pub outcome: ClientEchoOutcome,
    pub from: Option<String>,
    pub error: Option<ClientIcmpError>,
    pub duration_us: Option<u64>,
    pub setup: bool,
    pub path: Vec<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientIcmpError {
    pub kind: u8,
    pub code: u8,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientProbe {
    pub outcome: ClientProbeOutcome,
    pub duration_us: Option<u64>,
    pub started_at: std::time::SystemTime,
    pub completed_at: Option<std::time::SystemTime>,
    pub path: Vec<String>,
    pub transport: Option<String>,
    pub channels: Option<ClientProbeChannels>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientProbeChannels {
    pub request: ClientChannel,
    pub reply: Option<ClientChannel>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClientChannel {
    pub link_id: String,
    pub socket_id: String,
    pub peer_node_id: String,
    pub transport: String,
    pub local_address: String,
    pub remote_address: String,
    pub acquisition: ClientAcquisition,
}
enumeration!(
    ClientEndpointKind,
    ipc::EndpointKind,
    Configured,
    Interface,
    Stun
);
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ClientAcquisition {
    PublishedEndpoint,
    LocalEndpoint,
    StunEndpoint,
    AcceptedInbound,
    CoordinatedUdp {
        attempt_id: String,
        endpoint_source: ClientEndpointKind,
    },
}
impl From<meshway_model::observations::ProbeChannel> for ClientChannel {
    fn from(v: meshway_model::observations::ProbeChannel) -> Self {
        Self {
            link_id: v.link_id.to_string(),
            socket_id: v.socket_id.to_string(),
            peer_node_id: v.peer_node_id.to_string(),
            transport: v.transport.as_str().into(),
            local_address: v.local_address.to_string(),
            remote_address: v.remote_address.to_string(),
            acquisition: match v.acquisition {
                ipc::LinkAcquisition::PublishedEndpoint => ClientAcquisition::PublishedEndpoint,
                ipc::LinkAcquisition::LocalEndpoint => ClientAcquisition::LocalEndpoint,
                ipc::LinkAcquisition::StunEndpoint => ClientAcquisition::StunEndpoint,
                ipc::LinkAcquisition::AcceptedInbound => ClientAcquisition::AcceptedInbound,
                ipc::LinkAcquisition::CoordinatedUdp {
                    attempt_id,
                    endpoint_source,
                } => ClientAcquisition::CoordinatedUdp {
                    attempt_id: attempt_id.to_string(),
                    endpoint_source: endpoint_source.into(),
                },
            },
        }
    }
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ClientPingEvent {
    Decision {
        decision: ClientDecision,
    },
    Accepted {
        subscription_id: String,
        observation_id: String,
        target: ClientTarget,
    },
    Preparing {
        elapsed_ms: u64,
    },
    Sample {
        observation_id: String,
        measurement_id: String,
        version: u64,
        window_start: std::time::SystemTime,
        window_end: std::time::SystemTime,
        partial: bool,
        probe: Option<ClientProbe>,
    },
    Echo {
        echo: ClientEcho,
    },
    Completed {
        sent: u32,
        success: u32,
        timeout: u32,
        failed: u32,
    },
    Failed {
        code: ClientPingError,
    },
}
impl From<ipc::PingEvent> for ClientPingEvent {
    fn from(v: ipc::PingEvent) -> Self {
        match v {
            ipc::PingEvent::Decision { decision: v } => Self::Decision {
                decision: ClientDecision {
                    destination: v.destination,
                    node: v.node.map(Into::into),
                    profile: v.profile,
                    rule: v.rule.map(|v| ClientRule {
                        pointer: v.pointer,
                        remark: v.remark,
                        rule_set: v.rule_set.map(|v| ClientRuleSetMatch {
                            key: v.key,
                            entry: v.entry,
                        }),
                    }),
                    path: v.path.iter().map(ToString::to_string).collect(),
                    action: v.action.map(|v| match v {
                        ipc::PingAction::Mesh => ClientPingAction::Mesh,
                        ipc::PingAction::Exit { node } => ClientPingAction::Exit {
                            node: node.to_string(),
                        },
                        ipc::PingAction::Direct => ClientPingAction::Direct,
                        ipc::PingAction::Unmanaged => ClientPingAction::Unmanaged,
                        ipc::PingAction::Denied { reason } => ClientPingAction::Denied {
                            reason: reason.into(),
                        },
                    }),
                    resolution: v.resolution.map(|v| ClientResolution {
                        name: v.name,
                        resolver: match v.resolver {
                            ipc::PingResolver::MagicDns => ClientResolver::MagicDns,
                            ipc::PingResolver::Exit { node } => ClientResolver::Exit {
                                node: node.to_string(),
                            },
                            ipc::PingResolver::Resolvers { keys } => {
                                ClientResolver::Resolvers { keys }
                            }
                            ipc::PingResolver::System => ClientResolver::System,
                            ipc::PingResolver::Unavailable => ClientResolver::Unavailable,
                        },
                        addresses: v.addresses,
                        association: v.association.map(|v| ClientAssociation {
                            names: v.names,
                            remaining_seconds: v.remaining_seconds,
                        }),
                    }),
                },
            },
            ipc::PingEvent::Accepted {
                subscription_id,
                observation_id,
                target,
            } => Self::Accepted {
                subscription_id: subscription_id.to_string(),
                observation_id: observation_id.to_string(),
                target: target.into(),
            },
            ipc::PingEvent::Preparing { elapsed_ms } => Self::Preparing {
                elapsed_ms: elapsed_ms.get(),
            },
            ipc::PingEvent::Sample {
                observation_id,
                sample: v,
            } => Self::Sample {
                observation_id: observation_id.to_string(),
                measurement_id: v.measurement_id.to_string(),
                version: v.version.get(),
                window_start: v.window_start.into(),
                window_end: v.window_end.into(),
                partial: v.partial,
                probe: v.probe.map(|v| ClientProbe {
                    outcome: v.outcome.into(),
                    duration_us: v.duration_us.map(|v| v.get()),
                    started_at: v.started_at.into(),
                    completed_at: v.completed_at.map(Into::into),
                    path: v.path.iter().map(ToString::to_string).collect(),
                    transport: v.transport,
                    channels: v.channels.map(|v| ClientProbeChannels {
                        request: v.request.into(),
                        reply: v.reply.map(Into::into),
                    }),
                }),
            },
            ipc::PingEvent::Echo { echo: v } => Self::Echo {
                echo: ClientEcho {
                    sequence: v.sequence,
                    outcome: v.outcome.into(),
                    from: v.from,
                    error: v.error.map(|v| ClientIcmpError {
                        kind: v.kind,
                        code: v.code,
                    }),
                    duration_us: v.duration_us.map(|v| v.get()),
                    setup: v.setup,
                    path: v.path.iter().map(ToString::to_string).collect(),
                },
            },
            ipc::PingEvent::Completed {
                sent,
                success,
                timeout,
                failed,
            } => Self::Completed {
                sent,
                success,
                timeout,
                failed,
            },
            ipc::PingEvent::Failed { code } => Self::Failed { code: code.into() },
        }
    }
}
