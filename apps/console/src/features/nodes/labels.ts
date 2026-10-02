import type {
  ConnectionListenerState,
  ConnectionRuntimeReason,
  ConnectionRuntimeState,
  ConnectionTransport,
  EndpointDiscoverySource,
  LatencyFailureReason,
  StunDiscoveryOutcome,
} from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

import type { DiscoveryStatus, ObservationState } from "./discovery-model"

export const transportLabel: Record<ConnectionTransport, () => string> = {
  native_udp: () => m.node_preset_native_udp(),
  native_tcp: () => m.node_preset_native_tcp(),
  vless: () => m.node_transport_vless(),
}

export const runtimeStateLabel: Record<ConnectionRuntimeState, () => string> = {
  awaiting_report: () => m.node_runtime_awaiting_report(),
  disabled: () => m.node_runtime_disabled(),
  applying: () => m.node_runtime_applying(),
  applied: () => m.node_runtime_applied(),
  error: () => m.node_runtime_error(),
  withdrawn: () => m.node_runtime_withdrawn(),
}

export const listenerStateLabel: Record<ConnectionListenerState, () => string> =
  {
    bound: () => m.node_listener_bound(),
    error: () => m.node_listener_bind_error(),
  }

export const reasonLabel: Record<ConnectionRuntimeReason, () => string> = {
  none: () => m.node_reason_none(),
  disabled: () => m.node_reason_disabled(),
  invalid_configuration: () => m.node_reason_invalid_configuration(),
  profile_unavailable: () => m.node_reason_profile_unavailable(),
  bind_failed: () => m.node_reason_bind_failed(),
  resolve_failed: () => m.node_reason_resolve_failed(),
  transport_failed: () => m.node_reason_transport_failed(),
  authentication_failed: () => m.node_reason_authentication_failed(),
  probe_failed: () => m.node_reason_probe_failed(),
  lease_superseded: () => m.node_reason_lease_superseded(),
  lease_expired: () => m.node_reason_lease_expired(),
  shutting_down: () => m.node_reason_shutting_down(),
  internal_error: () => m.node_reason_internal_error(),
}

export const discoveryStatusLabel: Record<DiscoveryStatus, () => string> = {
  unreported: () => m.node_discovery_unreported(),
  outdated: () => m.node_discovery_outdated(),
  stale: () => m.node_discovery_stale(),
  awaiting_report: () => m.node_discovery_awaiting(),
  disabled: () => m.node_discovery_disabled(),
  probing: () => m.node_discovery_probing(),
  observed: () => m.node_discovery_observed(),
  withdrawn: () => m.node_discovery_withdrawn(),
  expired: () => m.node_discovery_expired(),
  error: () => m.node_discovery_error(),
}

export const observationLabel: Record<ObservationState, () => string> = {
  mapped: () => m.node_observation_mapped(),
  expired: () => m.node_observation_expired(),
  failed: () => m.node_observation_failed(),
}

export const stunOutcomeLabel: Record<StunDiscoveryOutcome, () => string> = {
  mapped: () => m.node_stun_mapped(),
  timeout: () => m.node_stun_timeout(),
  invalid_response: () => m.node_stun_invalid_response(),
  io_error: () => m.node_stun_io_error(),
  stun_error: () => m.node_stun_error(),
}

export const discoverySourceLabel: Record<
  EndpointDiscoverySource,
  () => string
> = {
  managed_udp_socket_stun: () => m.node_source_managed_udp_stun(),
}

export const latencyFailureLabel: Record<LatencyFailureReason, () => string> = {
  channel_unavailable: () => m.node_latency_channel_unavailable(),
  authentication_failed: () => m.node_latency_authentication_failed(),
  local_send_failed: () => m.node_latency_local_send_failed(),
}
