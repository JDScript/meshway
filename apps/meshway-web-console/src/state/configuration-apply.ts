import {
  applyConfiguration,
  validateConfiguration,
  type ConfigurationAppliedResponse,
  type ConfigurationPreviewResponse,
  type ConfigurationRequest,
  type ConfigurationValidationParams,
} from "@meshway/control-client"
import type { QueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import {
  ApiFailure,
  authed,
  client,
  problemCode,
  type Failure,
} from "@/api/client"
import { failureText, type FailureText } from "@/api/failure"
import {
  configurationQuery,
  configurationReads,
} from "@/api/queries/configuration"
import { networkKey, networksQuery } from "@/api/queries/network"
import type { DraftInput } from "@/lib/configuration/draft-state"
import { m } from "@/paraglide/messages.js"

import { attempts } from "./attempts"
import { draftActions } from "./configuration-draft"
import { onSessionChange } from "./session"

// Applying is always: validate, review what the controller says would
// change, then the person confirms. The draft (its JSONC document or the
// forms' typed operations) and a restore of a past revision take the same
// path; neither is ever applied unreviewed. Strict CAS: a newer revision
// keeps the draft and its base as they are.
//
// One flow per network, whichever page started it: the review and its
// outcome outlive the page (the network shell shows the dialogs), and an
// apply whose outcome is unknown is sent again as the same request.

const TOAST = "configuration-apply"

/** What is to be applied, without its request ID (one per distinct attempt). */
export type Proposal =
  | { type: "draft"; expectedRevision: number; input: DraftInput }
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

export type ApplyState = {
  /** A check or an apply is in flight; nothing else starts meanwhile. */
  busy: "checking" | "applying" | null
  review: Review | null
  refusal: Refusal | null
}

/** Where a refused draft is shown: the page that asked for the check says. */
export type OnInvalid = (
  input: DraftInput,
  params: ConfigurationValidationParams
) => void

const IDLE: ApplyState = { busy: null, review: null, refusal: null }

const applyStore = createStore<Readonly<Record<string, ApplyState>>>(() => ({}))

const applies = attempts<Proposal>()
/** The page that asked for each network's last check (a function: runtime, not state). */
const invalidHandlers = new Map<string, OnInvalid>()

onSessionChange(() => {
  applyStore.setState({}, true)
  invalidHandlers.clear()
})

const stateOf = (networkId: string) => applyStore.getState()[networkId] ?? IDLE

function patch(networkId: string, change: Partial<ApplyState>) {
  applyStore.setState({ [networkId]: { ...stateOf(networkId), ...change } })
}

export const useConfigurationApply = (networkId: string): ApplyState =>
  useStore(applyStore, (all) => all[networkId] ?? IDLE)

function withRequestId(
  proposal: Proposal,
  requestId: string
): ConfigurationRequest {
  const { expectedRevision } = proposal
  if (proposal.type === "restore")
    return {
      type: "restore",
      expectedRevision,
      restoreFrom: proposal.restoreFrom,
      requestId,
    }
  const { input } = proposal
  return input.kind === "document"
    ? { type: "document", expectedRevision, sourceJsonc: input.text, requestId }
    : { type: "operations", expectedRevision, ops: [...input.ops], requestId }
}

const failureOf = (error: unknown): Failure | null =>
  error instanceof ApiFailure ? error.failure : null

function invalidParams(
  failure: Failure | null
): ConfigurationValidationParams | null {
  if (failure?.kind !== "http") return null
  const problem = failure.problem?.problem
  return problem?.code === "configuration_invalid" ? problem.params : null
}

/**
 * A call's own words: a restore the controller cannot check says which
 * revision it was; a draft that cannot be checked or applied now may be
 * preparing remote rule sets over the controller's limits.
 */
const wordsFor = (proposal: Proposal): FailureText =>
  proposal.type === "restore"
    ? {
        service_unavailable: () =>
          m.configuration_restore_unavailable({
            revision: proposal.restoreFrom,
          }),
      }
    : { service_unavailable: () => m.configuration_draft_unavailable() }

/**
 * An apply's words. An attempt reuses its request ID only for the same
 * proposal, so the same ID with other input is not expected; if the
 * controller says so, nothing was applied and checking again starts a new
 * attempt. Distinct from a newer revision (`revision_conflict`).
 */
const applyWordsFor = (proposal: Proposal): FailureText => ({
  ...wordsFor(proposal),
  request_id_conflict: () => m.configuration_apply_request_conflict(),
})

/** The words for an apply's result, from what the controller returned. */
function appliedText(result: ConfigurationAppliedResponse): string {
  if (result.targetCount === 0)
    return m.configuration_applied_no_targets({ revision: result.revision })
  return m.configuration_applied_targets({
    revision: result.revision,
    targets: result.targetCount,
  })
}

async function refresh(queryClient: QueryClient, networkId: string) {
  await queryClient.invalidateQueries({
    queryKey: configurationReads(networkId),
  })
  await queryClient.invalidateQueries({ queryKey: networkKey(networkId) })
  void queryClient.invalidateQueries({ queryKey: networksQuery().queryKey })
}

/** The controller moved on: say so with both revisions; the draft stays. */
async function conflicted(
  queryClient: QueryClient,
  networkId: string,
  base: number
) {
  await refresh(queryClient, networkId)
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
}

function refused(
  networkId: string,
  proposal: Proposal,
  params: ConfigurationValidationParams
) {
  if (proposal.type === "restore") {
    patch(networkId, { refusal: { proposal, params } })
    return
  }
  const count = params.issues.length + params.requirements.length
  draftActions(networkId).checked({
    input: proposal.input,
    issues: params.issues,
    requirements: params.requirements,
  })
  toast.error(m.configuration_invalid_toast({ count }), { id: TOAST })
  invalidHandlers.get(networkId)?.(proposal.input, params)
}

/** Asks the controller what `proposal` would change, and opens the review. */
export async function checkConfiguration(
  queryClient: QueryClient,
  networkId: string,
  proposal: Proposal,
  onInvalid: OnInvalid
) {
  if (stateOf(networkId).busy !== null) return
  invalidHandlers.set(networkId, onInvalid)
  patch(networkId, { busy: "checking" })
  try {
    const preview = await authed((token) =>
      validateConfiguration(
        client,
        { networkId },
        withRequestId(proposal, crypto.randomUUID()),
        { token }
      )
    )
    if (proposal.type === "draft")
      draftActions(networkId).checked({
        input: proposal.input,
        issues: [],
        requirements: [],
      })
    patch(networkId, {
      refusal: null,
      review: { proposal, preview, unknown: false },
    })
  } catch (error) {
    const failure = failureOf(error)
    const code = failure ? problemCode(failure) : null
    const params = invalidParams(failure)
    if (code === "revision_conflict")
      await conflicted(queryClient, networkId, proposal.expectedRevision)
    else if (params) refused(networkId, proposal, params)
    else {
      const text = failureText(error, wordsFor(proposal))
      if (text) toast.error(text, { id: TOAST })
    }
  } finally {
    patch(networkId, { busy: null })
  }
}

/** Applies what was reviewed, against the revision it was reviewed at. */
export async function confirmConfiguration(
  queryClient: QueryClient,
  networkId: string,
  onShowProgress: (revision: number) => void
) {
  const { review, busy } = stateOf(networkId)
  if (!review || busy !== null) return
  const { proposal } = review
  patch(networkId, { busy: "applying" })
  try {
    const result = await applies.send(
      networkId,
      JSON.stringify(proposal),
      () => proposal,
      (attempt) =>
        authed((token) =>
          applyConfiguration(
            client,
            { networkId },
            withRequestId(attempt.body, attempt.requestId),
            { token }
          )
        )
    )
    if (proposal.type === "draft")
      draftActions(networkId).applied(proposal.input)
    patch(networkId, { review: null })
    toast.success(appliedText(result), {
      id: TOAST,
      action: {
        label: m.configuration_progress_open(),
        onClick: () => onShowProgress(result.revision),
      },
    })
    await refresh(queryClient, networkId)
  } catch (error) {
    const failure = failureOf(error)
    const code = failure ? problemCode(failure) : null
    const params = invalidParams(failure)
    if (applies.unknown(networkId)) {
      patch(networkId, { review: { ...review, unknown: true } })
      const text = failureText(error)
      if (text) toast.error(text, { id: TOAST })
    } else if (code === "revision_conflict") {
      patch(networkId, { review: null })
      await conflicted(queryClient, networkId, proposal.expectedRevision)
    } else if (params) {
      patch(networkId, { review: null })
      refused(networkId, proposal, params)
    } else {
      if (code === "request_id_conflict") patch(networkId, { review: null })
      const text = failureText(error, applyWordsFor(proposal))
      if (text) toast.error(text, { id: TOAST })
    }
  } finally {
    patch(networkId, { busy: null })
  }
}

export function closeReview(networkId: string) {
  if (stateOf(networkId).busy === null) patch(networkId, { review: null })
}

export function closeRefusal(networkId: string) {
  patch(networkId, { refusal: null })
}
