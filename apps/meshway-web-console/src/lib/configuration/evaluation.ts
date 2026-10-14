import type {
  AddressClass,
  ConfigurationView,
  EvaluationConfiguration,
  EvaluationTarget,
  TargetProblem,
  UnavailableReason,
} from "@meshway/control-client"

import { isIp } from "@/lib/ip"
import { m } from "@/paraglide/messages.js"

import { valueAt } from "./changes"
import { profileText, type NodeNamer } from "./diagnostics"
import type { DraftInput } from "./draft-state"
import { parseJsonc, pointerSegments } from "./jsonc"
import { valueOf } from "./operations"

// The path check (evaluation.md): what is typed as a destination, the
// configuration it is asked over (the current version or the draft), and
// the controller's answer in words. The controller decides everything; the
// rules it names are found in the configuration it was asked over only to
// say which rule each is.

/** A destination as typed: a node's hostname, an IP address or a domain. */
export function targetOf(
  typed: string,
  nodes: readonly { id: string; hostname: string }[]
): EvaluationTarget | null {
  const value = typed.trim()
  if (value === "") return null
  const node = nodes.find(
    (item) => item.hostname.toLowerCase() === value.toLowerCase()
  )
  if (node) return { type: "node", nodeId: node.id }
  if (isIp(value)) return { type: "address", address: value }
  // A name with a dot; the controller checks and normalizes it.
  return /^[^\s/:@]+\.[^\s/:@]+$/.test(value)
    ? { type: "domain", name: value }
    : null
}

/** The draft as the controller evaluates it: a change to the current version, at its base. */
export function draftConfiguration(
  baseRevision: number,
  input: DraftInput
): EvaluationConfiguration {
  return input.kind === "document"
    ? {
        type: "document",
        expectedRevision: baseRevision,
        sourceJsonc: input.text,
      }
    : {
        type: "operations",
        expectedRevision: baseRevision,
        ops: [...input.ops],
      }
}

/**
 * The draft's rules, for naming the ones an evaluation points at: the
 * source as typed, or the forms' values over their base. Null when the
 * source does not parse.
 */
export function draftRules(input: DraftInput): unknown {
  if (input.kind === "document") {
    const parsed = parseJsonc(input.text)
    return parsed.ok ? parsed.value : null
  }
  return {
    access: valueOf(input.ops, input.base, "access"),
    topology: valueOf(input.ops, input.base, "topology"),
    routing: valueOf(input.ops, input.base, "routing"),
  } satisfies Partial<ConfigurationView>
}

const index = (segment: string | undefined) =>
  segment !== undefined && /^(0|[1-9][0-9]*)$/.test(segment)
    ? Number(segment) + 1
    : null

/** Which rule a pointer names, as the forms number them. */
function ruleName(pointer: string, node: NodeNamer): string {
  const segments = pointerSegments(pointer) ?? []
  const [first, second, third, fourth, fifth, sixth] = segments
  const at = (segment: string | undefined) => index(segment) ?? 0
  if (segments.length === 2 && first === "access" && index(second))
    return m.nodes_path_rule_access({ index: at(second) })
  if (segments.length === 2 && first === "topology" && index(second))
    return m.nodes_path_rule_topology({ index: at(second) })
  if (segments.length === 3 && first === "routing" && second === "mesh") {
    if (index(third)) return m.nodes_path_rule_mesh({ index: at(third) })
  }
  if (first === "routing" && second === "egress") {
    if (segments.length === 4 && third === "rules" && index(fourth))
      return m.nodes_path_rule_public({ index: at(fourth) })
    if (segments.length === 4 && third === "assignments" && index(fourth))
      return m.nodes_path_rule_assignment({ index: at(fourth) })
    if (segments.length === 4 && third === "auto" && fourth === "fallback")
      return m.nodes_path_rule_auto_fallback()
    if (third === "profiles" && fourth !== undefined) {
      const profile = profileText(fourth, node)
      if (segments.length === 5 && fifth === "fallback")
        return m.nodes_path_rule_profile_fallback({ profile })
      if (segments.length === 6 && fifth === "rules" && index(sixth))
        return m.nodes_path_rule_profile({ profile, index: at(sixth) })
    }
  }
  return pointer
}

/** A rule the controller points at: which one, with its remark when the configuration gives one. */
export function ruleText(
  pointer: string,
  rules: unknown,
  node: NodeNamer
): { name: string; remark: string | null } {
  const found = valueAt(rules, pointer)
  const remark =
    found.found &&
    typeof found.value === "object" &&
    found.value !== null &&
    "remark" in found.value &&
    typeof found.value.remark === "string" &&
    found.value.remark.trim() !== ""
      ? found.value.remark
      : null
  return { name: ruleName(pointer, node), remark }
}

export const UNAVAILABLE_REASON: Record<UnavailableReason, () => string> = {
  unauthorized: () => m.nodes_path_reason_unauthorized(),
  noPath: () => m.nodes_path_reason_noPath(),
  routingCycle: () => m.nodes_path_reason_routingCycle(),
  routingLimit: () => m.nodes_path_reason_routingLimit(),
  missingDependency: () => m.nodes_path_reason_missingDependency(),
  exitUnavailable: () => m.nodes_path_reason_exitUnavailable(),
  unsupportedFamily: () => m.nodes_path_reason_unsupportedFamily(),
  egressConflict: () => m.nodes_path_reason_egressConflict(),
  inactiveNode: () => m.nodes_path_reason_inactiveNode(),
}

export const TARGET_PROBLEM: Record<TargetProblem, () => string> = {
  disabled: () => m.nodes_path_problem_disabled(),
  exitDisabled: () => m.nodes_path_problem_exitDisabled(),
  exitUnavailable: () => m.nodes_path_problem_exitUnavailable(),
  unsupportedFamily: () => m.nodes_path_problem_unsupportedFamily(),
  notEnrolled: () => m.nodes_path_problem_notEnrolled(),
  noPath: () => m.nodes_path_reason_noPath(),
  routingCycle: () => m.nodes_path_reason_routingCycle(),
  routingLimit: () => m.nodes_path_reason_routingLimit(),
}

export const ADDRESS_CLASS: Record<AddressClass, () => string> = {
  mesh: () => m.nodes_path_class_mesh(),
  private: () => m.nodes_path_class_private(),
  public: () => m.nodes_path_class_public(),
}
