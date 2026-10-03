import { useMatch, useNavigate } from "@tanstack/react-router"
import { useMemo } from "react"

import { useSurfaceHistory } from "@/features/shell/history"

import {
  isPolicyLayer,
  type EgressSection,
  type PolicyLayer,
  type PolicyWorkspaceView,
} from "./url"

const PAGE = "/_authed/networks/$network/policy" as const
const LAYER = "/_authed/networks/$network/policy/$layer" as const

/** The policy page as the URL has it. */
export function usePolicyRoute() {
  const current =
    useMatch({ from: PAGE, shouldThrow: false, select: () => true }) ?? false
  const match = useMatch({
    from: LAYER,
    shouldThrow: false,
    select: (match) => ({
      layer: match.params.layer,
      rule: match.search.rule ?? null,
      section: match.search.section ?? "public",
    }),
  })
  const layer: PolicyLayer | null =
    match && isPolicyLayer(match.layer) ? match.layer : null
  const view: PolicyWorkspaceView = {
    rule: match?.rule ?? null,
    section: match?.section ?? "public",
  }
  return { current, layer, view }
}

export function usePolicyNavigation(networkId: string) {
  const navigate = useNavigate()
  const history = useSurfaceHistory(networkId)
  const {
    layer: open,
    view: { rule, section },
  } = usePolicyRoute()
  return useMemo(() => {
    const params = { network: networkId }
    const view: PolicyWorkspaceView = { rule, section }
    const search = (next: PolicyWorkspaceView) => ({
      rule: next.rule ?? undefined,
      section: next.section === "public" ? undefined : next.section,
    })
    const within = (next: Partial<PolicyWorkspaceView>) => {
      if (!open) return
      void navigate({
        to: "/networks/$network/policy/$layer",
        params: { ...params, layer: open },
        search: search({ ...view, ...next }),
        ...history.keep,
      })
    }
    return {
      /** A layer's workspace, at a rule if given. */
      openLayer(layer: PolicyLayer, rule: string | null = null) {
        void navigate({
          to: "/networks/$network/policy/$layer",
          params: { ...params, layer },
          search: search({ rule, section: "public" }),
          ...history.open(open !== null),
        })
      },
      closeLayer() {
        history.close(
          () =>
            void navigate({
              to: "/networks/$network/policy",
              params,
              replace: true,
            })
        )
      },
      setRule: (rule: string | null) => within({ rule }),
      setSection: (section: EgressSection) => within({ section, rule: null }),
    }
  }, [navigate, history, networkId, open, rule, section])
}

export type PolicyNavigation = ReturnType<typeof usePolicyNavigation>

/** How long a jump's request to focus its rule waits for the rule to be shown. */
const RULE_FOCUS_MS = 5_000
let ruleFocus: { id: string; at: number } | null = null

/** The rule `id`, the next time its layer shows it as the one the address points at, takes focus once (a jump from the palette). */
export function requestRuleFocus(id: string) {
  ruleFocus = { id, at: Date.now() }
}

/** Whether the rule `id` should take focus now; true at most once per request. */
export function takeRuleFocus(id: string): boolean {
  const request = ruleFocus
  if (!request || Date.now() - request.at > RULE_FOCUS_MS) {
    ruleFocus = null
    return false
  }
  if (request.id !== id) return false
  ruleFocus = null
  return true
}
