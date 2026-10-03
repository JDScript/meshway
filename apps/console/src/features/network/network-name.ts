import type { NetworkNameRules } from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useCallback } from "react"

import { authOptionsQuery } from "@/features/auth/queries"
import {
  checkNetworkName,
  trimWhiteSpace,
  type NetworkNameProblem,
} from "@/lib/network-name"
import { m } from "@/paraglide/messages.js"

const problemText: Record<
  NetworkNameProblem,
  (rules: NetworkNameRules) => string
> = {
  empty: () => m.network_name_missing(),
  control: () => m.network_name_control(),
  bytes: (rules) =>
    m.network_name_bytes({ max: String(rules.maxInputUtf8Bytes) }),
  length: (rules) =>
    m.network_name_length({
      min: String(rules.minCodepoints),
      max: String(rules.maxCodepoints),
    }),
}

export type NameCheck = { name: string } | { refused: string }

/**
 * Checks a typed network name by the rules the controller publishes. Until
 * they are read only an empty name is refused here; the controller checks
 * again either way.
 */
export function useNetworkNameCheck() {
  const rules = useQuery(authOptionsQuery()).data?.networkNameRules ?? null
  return useCallback(
    (raw: string): NameCheck => {
      if (!rules) {
        const name = trimWhiteSpace(raw)
        return name ? { name } : { refused: m.network_name_missing() }
      }
      const checked = checkNetworkName(raw, rules)
      return checked.ok
        ? { name: checked.name }
        : { refused: problemText[checked.problem](rules) }
    },
    [rules]
  )
}
