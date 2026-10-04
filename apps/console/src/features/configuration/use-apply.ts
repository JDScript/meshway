import {
  applyConfiguration,
  validateConfiguration,
  type ConfigurationAppliedResponse,
  type ConfigurationPreviewResponse,
  type ConfigurationRequest,
  type ConfigurationValidationParams,
} from "@meshway/control-client"
import { useQueryClient } from "@tanstack/react-query"
import { useCallback, useRef, useState } from "react"
import { toast } from "sonner"

import { networkKey, networksQuery } from "@/features/network/queries"
import {
  ApiFailure,
  authed,
  client,
  problemCode,
  type Failure,
} from "@/lib/api"
import { attemptFor, outcomeUnknown, type Attempt } from "@/lib/attempt"
import { failureText, type FailureText } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { recordApplied } from "./applied"
import { useConfigurationDraft } from "./draft"
import { deploymentOf } from "./progress"
import { configurationQuery, configurationReads } from "./queries"

// Applying is always: validate, review what the controller says would
// change, then the person confirms. The draft's document and a restore of
// a past revision take the same path; neither is ever applied unreviewed.
// Strict CAS: a newer revision keeps the draft and its base as they are.

const TOAST = "configuration-apply"

/** What is to be applied, without its request ID (one per distinct attempt). */
export type Proposal =
  | { type: "document"; expectedRevision: number; sourceJsonc: string }
  | { type: "restore"; expectedRevision: number; restoreFrom: number }

export type Review = {
  proposal: Proposal
  preview: ConfigurationPreviewResponse
  /** The last apply's outcome is unknown: confirming again repeats it. */
  unknown: boolean
}

/** A restore the controller refused, with what it said. */
export type Refusal = {
  proposal: Proposal
  params: ConfigurationValidationParams
}

const withRequestId = (
  proposal: Proposal,
  requestId: string
): ConfigurationRequest =>
  proposal.type === "document"
    ? { ...proposal, requestId }
    : { ...proposal, requestId }

const failureOf = (error: unknown): Failure | null =>
  error instanceof ApiFailure ? error.failure : null

function invalidParams(
  failure: Failure | null
): ConfigurationValidationParams | null {
  if (failure?.kind !== "http") return null
  const problem = failure.problem?.problem
  return problem?.code === "configuration_invalid" ? problem.params : null
}

/** A restore the controller cannot check says which revision it was. */
const wordsFor = (proposal: Proposal): FailureText =>
  proposal.type === "restore"
    ? {
        service_unavailable: () =>
          m.configuration_restore_unavailable({
            revision: proposal.restoreFrom,
          }),
      }
    : {}

/** The words for an apply's result, from what the controller returned. */
export function appliedText(result: ConfigurationAppliedResponse): string {
  const deployment = deploymentOf(result)
  if (deployment === "none")
    return m.configuration_applied_no_targets({ revision: result.revision })
  if (deployment === "unpublished")
    return m.configuration_applied_targets_unpublished({
      revision: result.revision,
      targets: result.targetCount,
    })
  return m.configuration_applied_targets({
    revision: result.revision,
    targets: result.targetCount,
    publications: result.publicationIds.length,
  })
}

export function useConfigurationApply(
  networkId: string,
  {
    onInvalidDocument,
    onShowProgress,
  }: {
    onInvalidDocument: () => void
    /** Opens the deployment of a revision this tab applied. */
    onShowProgress: (revision: number) => void
  }
) {
  const queryClient = useQueryClient()
  const draft = useConfigurationDraft(networkId)
  const [checking, setChecking] = useState(false)
  const [applying, setApplying] = useState(false)
  const [review, setReview] = useState<Review | null>(null)
  const [refusal, setRefusal] = useState<Refusal | null>(null)
  const unsettled = useRef<Attempt<ConfigurationRequest> | null>(null)
  const busy = useRef(false)

  const refresh = useCallback(async () => {
    await queryClient.invalidateQueries({
      queryKey: configurationReads(networkId),
    })
    await queryClient.invalidateQueries({ queryKey: networkKey(networkId) })
    void queryClient.invalidateQueries({ queryKey: networksQuery().queryKey })
  }, [queryClient, networkId])

  /** The controller moved on: say so with both revisions; the draft stays. */
  const conflicted = useCallback(
    async (base: number) => {
      await refresh()
      const current = await queryClient
        .fetchQuery(configurationQuery(networkId))
        .then((response) => response.revision)
        .catch(() => null)
      toast.error(
        current === null
          ? m.problem_revision_conflict()
          : m.configuration_conflict_toast({ current, base }),
        { id: TOAST }
      )
    },
    [refresh, queryClient, networkId]
  )

  const refused = useCallback(
    (proposal: Proposal, params: ConfigurationValidationParams) => {
      const count = params.issues.length + params.requirements.length
      if (proposal.type === "document") {
        draft.checked({
          text: proposal.sourceJsonc,
          issues: params.issues,
          requirements: params.requirements,
        })
        toast.error(m.configuration_invalid_toast({ count }), { id: TOAST })
        onInvalidDocument()
      } else setRefusal({ proposal, params })
    },
    [draft, onInvalidDocument]
  )

  /** Asks the controller what `proposal` would change, and opens the review. */
  const check = useCallback(
    async (proposal: Proposal) => {
      if (busy.current) return
      busy.current = true
      setChecking(true)
      try {
        const preview = await authed((token) =>
          validateConfiguration(
            client,
            { networkId },
            withRequestId(proposal, crypto.randomUUID()),
            { token }
          )
        )
        if (proposal.type === "document")
          draft.checked({
            text: proposal.sourceJsonc,
            issues: [],
            requirements: [],
          })
        setRefusal(null)
        setReview({ proposal, preview, unknown: false })
      } catch (error) {
        const failure = failureOf(error)
        const code = failure ? problemCode(failure) : null
        const params = invalidParams(failure)
        if (code === "revision_conflict")
          await conflicted(proposal.expectedRevision)
        else if (params) refused(proposal, params)
        else {
          const text = failureText(error, wordsFor(proposal))
          if (text) toast.error(text, { id: TOAST })
        }
      } finally {
        busy.current = false
        setChecking(false)
      }
    },
    [networkId, draft, conflicted, refused]
  )

  /** Applies what was reviewed, against the revision it was reviewed at. */
  const confirm = useCallback(async () => {
    if (!review || busy.current) return
    const { proposal } = review
    const attempt = attemptFor(
      unsettled.current,
      JSON.stringify(proposal),
      () => withRequestId(proposal, crypto.randomUUID())
    )
    busy.current = true
    setApplying(true)
    try {
      const result = await authed((token) =>
        applyConfiguration(client, { networkId }, attempt.body, { token })
      )
      unsettled.current = null
      if (proposal.type === "document") draft.applied(proposal.sourceJsonc)
      setReview(null)
      recordApplied(result)
      toast.success(appliedText(result), {
        id: TOAST,
        action: {
          label: m.configuration_progress_open(),
          onClick: () => onShowProgress(result.revision),
        },
      })
      await refresh()
    } catch (error) {
      const failure = failureOf(error)
      unsettled.current = failure && outcomeUnknown(failure) ? attempt : null
      const code = failure ? problemCode(failure) : null
      const params = invalidParams(failure)
      if (unsettled.current) {
        setReview({ ...review, unknown: true })
        const text = failureText(error)
        if (text) toast.error(text, { id: TOAST })
      } else if (code === "revision_conflict") {
        setReview(null)
        await conflicted(proposal.expectedRevision)
      } else if (params) {
        setReview(null)
        refused(proposal, params)
      } else {
        const text = failureText(error, wordsFor(proposal))
        if (text) toast.error(text, { id: TOAST })
      }
    } finally {
      busy.current = false
      setApplying(false)
    }
  }, [review, networkId, draft, refresh, conflicted, refused, onShowProgress])

  const closeReview = useCallback(() => {
    if (!busy.current) setReview(null)
  }, [])
  const closeRefusal = useCallback(() => setRefusal(null), [])

  return {
    checking,
    applying,
    review,
    refusal,
    check,
    confirm,
    closeReview,
    closeRefusal,
  }
}
