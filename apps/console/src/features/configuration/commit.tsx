import type {
  ConfigurationValidationParams,
  NetworkSummary,
} from "@meshway/control-client"
import { useQuery } from "@tanstack/react-query"
import { useMemo } from "react"

import { useCommit } from "@/features/shell/commit-bar"
import type { PageId } from "@/features/shell/pages"
import { m } from "@/paraglide/messages.js"

import { useConfigurationDraft } from "./draft"
import { isOutdated, type DraftInput } from "./draft-state"
import { operationsProblems } from "./form-problems"
import { parseJsonc } from "./jsonc"
import { configurationQuery } from "./queries"
import { RefusalDialog, ReviewDialog } from "./review-dialog"
import { useConfigurationApply, type Proposal } from "./use-apply"

/**
 * The configuration draft's commit bar, review and refusal, for whichever
 * page edits it (the configuration page, a node's workspace): one draft,
 * so one bar and one review wherever it was edited from. `active`: the
 * page is on screen and reads the configuration.
 */
export function useConfigurationCommit({
  networkId,
  network,
  page,
  active,
  onInvalid,
  onShowProgress,
}: {
  networkId: string
  network: NetworkSummary | null
  page: PageId
  active: boolean
  onInvalid: (input: DraftInput, params: ConfigurationValidationParams) => void
  onShowProgress: (revision: number) => void
}) {
  const draft = useConfigurationDraft(networkId)
  const configuration = useQuery({
    ...configurationQuery(networkId),
    enabled: active,
  })
  const current = configuration.data?.revision ?? network?.currentRevision
  const editable = network?.actions.editConfiguration ?? false
  const apply = useConfigurationApply(networkId, { onInvalid, onShowProgress })

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
  /** Checks the draft, against the controller's revision once it moved on (a new review). */
  const checkDraft = () => {
    if (!entry || blocked !== null) return
    void apply.check({
      type: "draft",
      input: entry.input,
      expectedRevision:
        outdated && current !== undefined ? current : entry.baseRevision,
    })
  }
  useCommit(
    active && editable && draft.dirty && entry
      ? {
          page,
          label: m.configuration_commit_label(),
          tone: outdated ? "warning" : blocked ? "destructive" : "plain",
          text:
            outdated && current !== undefined
              ? m.configuration_commit_outdated({ current })
              : blocked
                ? blocked
                : apply.checking
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
          busy: apply.checking,
          // Starting over reads the current configuration; discarding keeps the base.
          secondary: outdated ? draft.reset : draft.discard,
          primary: checkDraft,
        }
      : null
  )

  const dialogs = (
    <>
      <ReviewDialog
        networkId={networkId}
        review={apply.review}
        applying={apply.applying}
        onConfirm={() => void apply.confirm()}
        onClose={apply.closeReview}
      />
      <RefusalDialog refusal={apply.refusal} onClose={apply.closeRefusal} />
    </>
  )

  return {
    checking: apply.checking,
    checkDraft,
    check: (proposal: Proposal) => void apply.check(proposal),
    dialogs,
  }
}
