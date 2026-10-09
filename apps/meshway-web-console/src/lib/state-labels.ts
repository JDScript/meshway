import type {
  ApplyPhase,
  EndpointKind,
  EndpointSource,
  LinkAcquisition,
  LinkEvidence,
  LinkPhase,
  ListenerPhase,
  LocationAbsentReason,
  PathEvidence,
  ReporterRole,
  ResourceRef,
  StateProblem,
} from "@meshway/control-client"

import type { MapConnection } from "@/lib/network-map"
import type { ConnectionStatus } from "@/lib/nodes/connections"
import type { NodeStatus } from "@/lib/nodes/status"
import { m } from "@/paraglide/messages.js"

export const connectionStatusLabel: Record<ConnectionStatus, () => string> = {
  connected: () => linkPhaseLabel.authenticated(),
  connecting: () => linkPhaseLabel.preparing(),
  failed: () => linkPhaseLabel.failed(),
  unknown: () => statusLabel.unknown(),
}

export function mapConnectionText(connection: MapConnection): string {
  switch (connection.status) {
    case "allowed":
      return m.overview_allowed()
    case "unavailable":
      return m.overview_no_transports()
    case "history":
      return m.overview_history()
    default:
      return connectionStatusLabel[connection.status]()
  }
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
  datasetUnavailable: () => m.state_location_dataset_unavailable(),
  notInDataset: () => m.state_location_not_in_dataset(),
  lookupFailed: () => m.state_location_lookup_failed(),
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
const endpointKindLabel: Record<EndpointKind, () => string> = {
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
