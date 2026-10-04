import type { NetworkSummary } from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useMemo } from "react"

import { configurationQuery } from "@/api/queries/configuration"
import type { Commit } from "@/components/surface/commit-bar"
import { isOutdated } from "@/lib/configuration/draft-state"
import { operationsProblems } from "@/lib/configuration/form-problems"
import { parseJsonc } from "@/lib/configuration/jsonc"
import { m } from "@/paraglide/messages.js"
import {
  checkConfiguration,
  useConfigurationApply,
  type OnInvalid,
  type Proposal,
} from "@/state/configuration-apply"
import { useConfigurationDraft } from "@/state/configuration-draft"

/**
 * The configuration draft's commit bar, for whichever page edits it (the
 * configuration page, a node's workspace): one draft, so one bar and one
 * review wherever it was edited from. The review and refusal are the
 * network shell's (`ConfigurationDialogs`). `onInvalid` shows a refused
 * draft where this page edits it.
 */
export function useConfigurationCommit({
  networkId,
  network,
  onInvalid,
}: {
  networkId: string
  network: NetworkSummary | null
  onInvalid: OnInvalid
}) {
  const queryClient = useQueryClient()
  const draft = useConfigurationDraft(networkId)
  const configuration = useQuery(configurationQuery(networkId))
  const current = configuration.data?.revision ?? network?.currentRevision
  const editable = network?.actions.editConfiguration ?? false
  const checking = useConfigurationApply(networkId).busy === "checking"

  const entry = draft.entry
  const outdated = isOutdated(entry, current)
  const input = draft.dirty ? (entry?.input ?? null) : null
  // A document that does not parse, or form fields that will not save, stop applying.
  const blocked = useMemo(() => {
    if (input === null) return null
    if (input.kind === "document")
      return parseJsonc(input.text).ok ? null : m.configuration_commit_invalid()
    const count = operationsProblems(input.ops, input.base).length
    return count === 0 ? null : m.configuration_commit_fields({ count })
  }, [input])

  const check = (proposal: Proposal) =>
    void checkConfiguration(queryClient, networkId, proposal, onInvalid)
  /** Checks the draft, against the controller's revision once it moved on (a new review). */
  const checkDraft = () => {
    if (!entry || blocked !== null) return
    check({
      type: "draft",
      input: entry.input,
      expectedRevision:
        outdated && current !== undefined ? current : entry.baseRevision,
    })
  }

  const bar: Commit | null =
    editable && draft.dirty && entry
      ? {
          label: m.configuration_commit_label(),
          tone: outdated ? "warning" : blocked ? "destructive" : "plain",
          text:
            outdated && current !== undefined
              ? m.configuration_commit_outdated({ current })
              : blocked
                ? blocked
                : checking
                  ? m.configuration_checking()
                  : m.configuration_commit_unsaved(),
          secondaryLabel:
            outdated && current !== undefined
              ? m.configuration_commit_start_over({ current })
              : m.configuration_commit_discard(),
          primaryLabel:
            outdated && current !== undefined
              ? m.configuration_commit_recheck({ current })
              : m.configuration_commit_apply(),
          primaryDisabled: blocked !== null,
          busy: checking,
          // Starting over reads the current configuration; discarding keeps the base.
          secondary: outdated ? draft.reset : draft.discard,
          primary: checkDraft,
        }
      : null

  return { bar, checking, checkDraft, check }
}
