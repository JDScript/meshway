import type {
  ApplyPhase,
  EndpointKind,
  EndpointSource,
  ExitDnsStatus,
  ExitDnsUnavailableReason,
  ExitStatus,
  ExitUnavailableReason,
  LinkAcquisition,
  LinkEvidence,
  LinkPhase,
  ListenerPhase,
  LocationAbsentReason,
  NetmapRef,
  PathEvidence,
  PinnedServerKeyStatus,
  ReporterRole,
  ResourceRef,
  StateProblem,
} from "@meshway/control-client"

import type { ConnectionStatus } from "@/lib/nodes/connections"
import type { NodeStatus } from "@/lib/nodes/status"
import { m } from "@/paraglide/messages.js"

export const connectionStatusLabel: Record<ConnectionStatus, () => string> = {
  connected: () => linkPhaseLabel.authenticated(),
  connecting: () => linkPhaseLabel.preparing(),
  failed: () => linkPhaseLabel.failed(),
  unknown: () => statusLabel.unknown(),
}

/** Known carrier names use the same wording as configuration; future providers keep their name. */
export function transportText(transport: string): string {
  switch (transport) {
    case "native_udp":
      return m.configuration_dialer_native_udp()
    case "native_tcp":
      return m.configuration_dialer_native_tcp()
    case "vless":
      return m.configuration_dialer_vless()
    case "trojan":
      return m.configuration_dialer_trojan()
    default:
      return transport
  }
}

// The words for runtime State, one per value the controller sends, the
// same on every page that shows it.

/** A node's status (`lib/nodes/status.ts`). */
export const statusLabel: Record<NodeStatus, () => string> = {
  current: () => m.state_status_current(),
  awaiting_report: () => m.state_status_awaiting_report(),
  superseded: () => m.state_status_superseded(),
  config_mismatch: () => m.state_status_config_mismatch(),
  expired: () => m.state_status_expired(),
  revoked: () => m.state_status_revoked(),
  disabled: () => m.state_status_disabled(),
  unknown: () => m.state_status_unknown(),
}

export const listenerPhaseLabel: Record<ListenerPhase, () => string> = {
  preparing: () => m.state_listener_phase_preparing(),
  listening: () => m.state_listener_phase_listening(),
  stopped: () => m.state_listener_phase_stopped(),
  failed: () => m.state_listener_phase_failed(),
}

/** A node's netmap in short: the revision it comes from and its serial. */
export const netmapText = (netmap: NetmapRef) =>
  m.netmap_ref({ revision: netmap.revision, serial: netmap.serial })

const exitUnavailableLabel: Record<ExitUnavailableReason, () => string> = {
  ipForwardDisabled: () => m.state_exit_ip_forward_disabled(),
  forwardingProbeFailed: () => m.state_exit_forwarding_probe_failed(),
  netAdminMissing: () => m.state_exit_net_admin_missing(),
  nftUnavailable: () => m.state_exit_nft_unavailable(),
  conntrackUnavailable: () => m.state_exit_conntrack_unavailable(),
  reversePathFilterConflict: () => m.state_exit_reverse_path_filter(),
  tunUnavailable: () => m.state_exit_tun_unavailable(),
  firewallConflict: () => m.state_exit_firewall_conflict(),
  routeConflict: () => m.state_exit_route_conflict(),
  unsupportedFamily: () => m.state_exit_unsupported_family(),
  ownershipConflict: () => m.state_exit_ownership_conflict(),
}

/** An exit's lifecycle as the node observed it, with the cause when it cannot serve. */
export function exitStatusText(status: ExitStatus): string {
  switch (status.status) {
    case "notRequested":
      return m.state_exit_not_requested()
    case "probing":
      return m.state_exit_probing()
    case "ready":
      return m.state_exit_ready()
    case "unavailable":
      return m.state_exit_unavailable({
        reason: exitUnavailableLabel[status.reason](),
      })
  }
}

const exitDnsReasonLabel: Record<ExitDnsUnavailableReason, () => string> = {
  unobserved: () => m.state_exit_dns_unobserved(),
  noServers: () => m.state_exit_dns_no_servers(),
  localServer: () => m.state_exit_dns_local_server(),
  anchorServer: () => m.state_exit_dns_anchor_server(),
  meshServer: () => m.state_exit_dns_mesh_server(),
  exitRoutedServer: () => m.state_exit_dns_exit_routed_server(),
  proxyStubMissing: () => m.state_exit_dns_proxy_stub_missing(),
}

/** Whether the exit resolves its sources' names, with the cause when it cannot. */
export function exitDnsText(status: ExitDnsStatus): string {
  return status.status === "serving"
    ? m.state_exit_dns_serving()
    : m.state_exit_dns_unavailable({
        reason: exitDnsReasonLabel[status.reason](),
      })
}

export const serverKeyLabel: Record<PinnedServerKeyStatus, () => string> = {
  current: () => m.state_server_key_current(),
  rotating: () => m.state_server_key_rotating(),
  keyMismatch: () => m.state_server_key_mismatch(),
}

export const applyPhaseLabel: Record<ApplyPhase, () => string> = {
  receiving: () => m.state_apply_receiving(),
  validating: () => m.state_apply_validating(),
  saved: () => m.state_apply_saved(),
  applying: () => m.state_apply_applying(),
  applied: () => m.state_apply_applied(),
  failed: () => m.state_apply_failed(),
}

export const linkPhaseLabel: Record<LinkPhase, () => string> = {
  preparing: () => m.state_link_preparing(),
  authenticated: () => m.state_link_authenticated(),
  closed: () => m.state_link_closed(),
  failed: () => m.state_link_failed(),
}

export const evidenceLabel: Record<LinkEvidence, () => string> = {
  oneSided: () => m.state_evidence_one_sided(),
  matched: () => m.state_evidence_matched(),
  conflicting: () => m.state_evidence_conflicting(),
  expired: () => m.state_evidence_expired(),
}

export const pathEvidenceLabel: Record<PathEvidence, () => string> = {
  selected: () => m.state_path_selected(),
  sent: () => m.state_path_sent(),
  peerConfirmed: () => m.state_path_peer_confirmed(),
  delivered: () => m.state_path_delivered(),
}

export const reporterRoleLabel: Record<ReporterRole, () => string> = {
  origin: () => m.state_role_origin(),
  relay: () => m.state_role_relay(),
  target: () => m.state_role_target(),
  exit: () => m.state_role_exit(),
}

export const locationAbsentLabel: Record<LocationAbsentReason, () => string> = {
  noObservation: () => m.state_location_no_observation(),
  observationExpired: () => m.state_location_observation_expired(),
  ambiguousObservation: () => m.state_location_ambiguous(),
}

/** Where an endpoint candidate came from, with what it names. */
export function endpointSourceText(origin: EndpointSource): string {
  switch (origin.source) {
    case "configured":
      return m.state_endpoint_configured()
    case "interface":
      return m.state_endpoint_interface({ name: origin.interface })
    case "stun":
      return m.state_endpoint_stun({ observer: origin.observer })
  }
}

/** An endpoint's source without its details (a coordinated attempt names only the kind). */
export const endpointKindLabel: Record<EndpointKind, () => string> = {
  configured: () => m.state_endpoint_configured(),
  interface: () => m.state_endpoint_kind_interface(),
  stun: () => m.state_endpoint_kind_stun(),
}

/** How a node acquired one of its channels, as it reports it. */
export function acquisitionText(acquisition: LinkAcquisition): string {
  switch (acquisition.type) {
    case "publishedEndpoint":
      return m.state_acquisition_published()
    case "localEndpoint":
      return m.state_acquisition_local()
    case "stunEndpoint":
      return m.state_acquisition_stun()
    case "acceptedInbound":
      return m.state_acquisition_inbound()
    case "coordinatedUdp":
      return m.state_acquisition_coordinated({
        source: endpointKindLabel[acquisition.endpointSource](),
      })
  }
}

export function resourceText(resource: ResourceRef): string {
  switch (resource.kind) {
    case "listener":
      return m.state_resource_listener({ key: resource.key })
    case "dial":
      return m.state_resource_dial({ key: resource.key })
    case "coordination":
      return m.state_resource_coordination({ key: resource.key })
  }
}

/** One problem the node reported, in words. */
export function problemText(problem: StateProblem): string {
  switch (problem.code) {
    case "resource_unavailable":
      return m.state_problem_resource_unavailable({
        resource: resourceText(problem.resource),
      })
    case "configuration_rejected":
      return m.state_problem_configuration_rejected({
        revision: problem.target.revision,
      })
    case "probe_failed":
      return m.state_problem_probe_failed({
        id: problem.measurementId.slice(0, 8),
      })
  }
}
