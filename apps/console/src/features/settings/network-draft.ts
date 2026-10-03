import type {
  NetworkSnapshot,
  SaveNetworkRequest,
} from "@meshway/control-client"
import {
  zMeshDnsConfiguration,
  zSaveNetworkRequest,
} from "@meshway/control-client/schemas"

import { createDraft, type Saved } from "@/features/shell/drafts"
import { m } from "@/paraglide/messages.js"

/** The network's settings as they are edited: text as typed, turned into the request when saved. */
export type NetworkFields = {
  name: string
  dnsEnabled: boolean
  zone: string
  ttl: string
}

export const useNetworkDraft = createDraft<NetworkFields>("settings")

export function savedNetwork(snapshot: NetworkSnapshot): Saved<NetworkFields> {
  return {
    revision: snapshot.revision,
    value: {
      name: snapshot.name,
      dnsEnabled: snapshot.dns.enabled,
      zone: snapshot.dns.zone,
      ttl: String(snapshot.dns.ttl_seconds),
    },
  }
}

const TTL = /^[0-9]{1,3}$/

type NetworkField = "name" | "zone" | "ttl"

/** The fields the generated schema rejects, in form order; the controller still checks the rest. */
export function invalidNetworkFields(fields: NetworkFields): NetworkField[] {
  const invalid: NetworkField[] = []
  if (!zSaveNetworkRequest.shape.name.safeParse(fields.name).success)
    invalid.push("name")
  if (!zMeshDnsConfiguration.shape.zone.safeParse(fields.zone).success)
    invalid.push("zone")
  if (
    !TTL.test(fields.ttl) ||
    !zMeshDnsConfiguration.shape.ttl_seconds.safeParse(Number(fields.ttl))
      .success
  )
    invalid.push("ttl")
  return invalid
}

export const networkFieldProblem: Record<NetworkField, () => string> = {
  name: () => m.settings_name_invalid(),
  zone: () => m.settings_zone_invalid(),
  ttl: () => m.settings_ttl_invalid(),
}

export function networkRequest(
  fields: NetworkFields,
  revision: number
): SaveNetworkRequest {
  return {
    expected_revision: revision,
    name: fields.name,
    dns: {
      enabled: fields.dnsEnabled,
      zone: fields.zone,
      ttl_seconds: Number(fields.ttl),
    },
  }
}
