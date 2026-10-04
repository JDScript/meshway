import type {
  ConfigurationOperation,
  ConfigurationView,
} from "@meshway/control-client"

import { addressFamily, inPools, sameAddress } from "./addresses"
import { dialerProblems } from "./dialer-model"
import type { LocalProblem } from "./form-problems"
import { pointerOf } from "./jsonc"
import {
  addressesOf,
  isDeleted,
  nodeOps,
  settingsOf,
  tagsOf,
} from "./node-operations"
import { valueOf } from "./operations"

// What a node's edits cannot be saved with, found before asking the
// controller (nodes.md, identity.md, addressing.md), at the pointers the
// controller would use. Only the nodes the draft edits are checked; the
// controller checks every node again.

/** One DNS label, lowercase (nodes.md#hostname). */
const HOSTNAME = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/

export function nodeProblems(
  ops: readonly ConfigurationOperation[],
  base: ConfigurationView
): LocalProblem[] {
  const out: LocalProblem[] = []
  const ids = Object.keys(base.nodes).filter((id) => !isDeleted(ops, id))
  const edited = ids.filter((id) => nodeOps(ops, id).length > 0)
  const owners = valueOf(ops, base, "tagsOwners")
  const pools = valueOf(ops, base, "addressPools")

  for (const nodeId of edited) {
    const at = (...path: (string | number)[]) =>
      pointerOf(["nodes", nodeId, ...path])
    const settings = settingsOf(ops, base, nodeId)
    if (settings) {
      if (!HOSTNAME.test(settings.hostname))
        out.push({ path: at("hostname"), code: "hostname" })
      else if (
        ids.some(
          (other) =>
            other !== nodeId &&
            settingsOf(ops, base, other)?.hostname === settings.hostname
        )
      )
        out.push({ path: at("hostname"), code: "hostname_taken" })
      if (settings.dialers)
        for (const protocol of dialerProblems(settings.dialers))
          out.push({ path: at("dialers", protocol), code: "dialer" })
    }

    const tagOp = nodeOps(ops, nodeId).some(
      (op) => op.type === "setIdentityTags"
    )
    const tags = tagOp ? (tagsOf(ops, base, nodeId) ?? []) : []
    if (tagOp && tags.length === 0)
      out.push({ path: at("identity", "tags"), code: "tags" })
    else if (tags.some((tag) => !Object.hasOwn(owners, tag)))
      out.push({ path: at("identity", "tags"), code: "tag" })

    if (nodeOps(ops, nodeId).some((op) => op.type === "setAddresses")) {
      const addresses = addressesOf(ops, base, nodeId)
      for (const family of ["ipv4", "ipv6"] as const) {
        const value = addresses?.[family] ?? null
        if (value === null) continue
        const path = at("addresses", family)
        if (addressFamily(value) !== family) out.push({ path, code: "address" })
        else if (!inPools(value, pools[family]))
          out.push({ path, code: "address_pool" })
        else if (
          ids.some((other) => {
            if (other === nodeId) return false
            const theirs = addressesOf(ops, base, other)?.[family] ?? null
            return theirs !== null && sameAddress(theirs, value)
          })
        )
          out.push({ path, code: "address_taken" })
      }
    }
  }
  return out
}
