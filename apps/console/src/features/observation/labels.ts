import type {
  MetricKind,
  ObservationPhase,
  ObservationServiceScope,
} from "@meshway/control-client"

import { m } from "@/paraglide/messages.js"

export const phaseLabel: Record<ObservationPhase, () => string> = {
  preparing: () => m.observation_phase_preparing(),
  running: () => m.observation_phase_running(),
  completed: () => m.observation_phase_completed(),
  failed: () => m.observation_phase_failed(),
  expired: () => m.observation_phase_expired(),
  cancelled: () => m.observation_phase_cancelled(),
  unsupported: () => m.observation_phase_unsupported(),
}

export const metricLabel: Record<MetricKind, () => string> = {
  rtt: () => m.observation_metric_rtt(),
  probeLoss: () => m.observation_metric_probe_loss(),
  traffic: () => m.observation_metric_traffic(),
}

export function serviceText(service: ObservationServiceScope): string {
  switch (service.type) {
    case "icmpEcho":
      return m.observation_service_icmp()
    case "tcp":
      return m.observation_service_tcp_port({ port: String(service.port) })
    case "udp":
      return m.observation_service_udp_port({ port: String(service.port) })
  }
}
