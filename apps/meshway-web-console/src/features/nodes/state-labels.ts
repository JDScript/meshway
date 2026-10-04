import type {
  ApplyPhase,
  EndpointSource,
  Freshness,
  LinkEvidence,
  LinkPhase,
  ListenerPhase,
  LocationAbsentReason,
  PathEvidence,
  ReporterRole,
  ResourceRef,
  StateProblem,
} from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

// The words for runtime State, one per value the controller sends.

export const freshnessLabel: Record<Freshness, () => string> = {
  current: () => m.nodes_freshness_current(),
  awaiting_report: () => m.nodes_freshness_awaiting_report(),
  expired: () => m.nodes_freshness_expired(),
  superseded: () => m.nodes_freshness_superseded(),
  config_mismatch: () => m.nodes_freshness_config_mismatch(),
  revoked: () => m.nodes_freshness_revoked(),
}

export const listenerPhaseLabel: Record<ListenerPhase, () => string> = {
  preparing: () => m.nodes_listener_phase_preparing(),
  listening: () => m.nodes_listener_phase_listening(),
  stopped: () => m.nodes_listener_phase_stopped(),
  failed: () => m.nodes_listener_phase_failed(),
}

export const applyPhaseLabel: Record<ApplyPhase, () => string> = {
  receiving: () => m.nodes_apply_receiving(),
  validating: () => m.nodes_apply_validating(),
  saved: () => m.nodes_apply_saved(),
  applying: () => m.nodes_apply_applying(),
  applied: () => m.nodes_apply_applied(),
  failed: () => m.nodes_apply_failed(),
}

export const linkPhaseLabel: Record<LinkPhase, () => string> = {
  preparing: () => m.nodes_link_preparing(),
  authenticated: () => m.nodes_link_authenticated(),
  closed: () => m.nodes_link_closed(),
  failed: () => m.nodes_link_failed(),
}

export const evidenceLabel: Record<LinkEvidence, () => string> = {
  oneSided: () => m.nodes_evidence_one_sided(),
  matched: () => m.nodes_evidence_matched(),
  conflicting: () => m.nodes_evidence_conflicting(),
  expired: () => m.nodes_evidence_expired(),
}

export const pathEvidenceLabel: Record<PathEvidence, () => string> = {
  selected: () => m.nodes_path_selected(),
  sent: () => m.nodes_path_sent(),
  peerConfirmed: () => m.nodes_path_peer_confirmed(),
  delivered: () => m.nodes_path_delivered(),
}

export const reporterRoleLabel: Record<ReporterRole, () => string> = {
  origin: () => m.nodes_role_origin(),
  relay: () => m.nodes_role_relay(),
  target: () => m.nodes_role_target(),
  exit: () => m.nodes_role_exit(),
}

export const locationAbsentLabel: Record<LocationAbsentReason, () => string> = {
  noObservation: () => m.nodes_location_no_observation(),
  observationExpired: () => m.nodes_location_observation_expired(),
  datasetUnavailable: () => m.nodes_location_dataset_unavailable(),
  notInDataset: () => m.nodes_location_not_in_dataset(),
  lookupFailed: () => m.nodes_location_lookup_failed(),
  ambiguousObservation: () => m.nodes_location_ambiguous(),
}

/** Where an endpoint candidate came from, with what it names. */
export function endpointSourceText(origin: EndpointSource): string {
  switch (origin.source) {
    case "configured":
      return m.nodes_endpoint_configured()
    case "interface":
      return m.nodes_endpoint_interface({ name: origin.interface })
    case "stun":
      return m.nodes_endpoint_stun({ observer: origin.observer })
  }
}

export function resourceText(resource: ResourceRef): string {
  switch (resource.kind) {
    case "listener":
      return m.nodes_resource_listener({ key: resource.key })
    case "dial":
      return m.nodes_resource_dial({ key: resource.key })
    case "coordination":
      return m.nodes_resource_coordination({ key: resource.key })
  }
}

/** One problem the node reported, in words. */
export function problemText(problem: StateProblem): string {
  switch (problem.code) {
    case "resource_unavailable":
      return m.nodes_problem_resource_unavailable({
        resource: resourceText(problem.resource),
      })
    case "configuration_rejected":
      return m.nodes_problem_configuration_rejected({
        revision: problem.target.revision,
      })
    case "probe_failed":
      return m.nodes_problem_probe_failed({
        id: problem.measurementId.slice(0, 8),
      })
  }
}
