import {
  saveNetworkPolicy,
  type NetworkPolicy,
  type NetworkPolicySnapshot,
} from "@meshway/control-client"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback, useMemo } from "react"

import { networkQuery, networksQuery } from "@/features/network/queries"
import { useCanManage } from "@/features/network/role"
import { useDraftCommit } from "@/features/shell/draft-commit"
import { createDraft, type Saved } from "@/features/shell/drafts"
import { authed } from "@/lib/api"
import { sameData } from "@/lib/same-data"

import {
  editPolicy,
  movePolicy,
  placeDiagnostic,
  policyText,
  readDocument,
  type PlacedDiagnostic,
  type PolicyDocument,
} from "./document"
import type { JsonPath } from "./jsonc"
import { problemMessage } from "./problems"
import { policyQuery } from "./queries"

/**
 * The policy draft is its JSONC text: the graphical views and the source
 * editor read and write the same text, against one revision. A network
 * still on the v2 source has no text: its saved value is empty until the
 * migration proposal or the template is put into the draft.
 */
export const usePolicyDraft = createDraft<string>("policy")

export function savedPolicy(snapshot: NetworkPolicySnapshot): Saved<string> {
  const text =
    snapshot.source_mode === "mesh_v3"
      ? (snapshot.source_jsonc ??
        (snapshot.policy ? policyText(snapshot.policy) : ""))
      : ""
  return { revision: snapshot.revision, value: text }
}

/** Where the policy the views show comes from. */
export type PolicyOrigin = "saved" | "draft" | "proposal"

export const POLICY_SECTIONS = [
  "connections",
  "grants",
  "mesh",
  "public_rules",
  "profiles",
  "automatic",
  "assignments",
] as const satisfies readonly (keyof NetworkPolicy)[]

/** The policy page's state, shared by its panel and workspaces. */
export function usePolicy(networkId: string, enabled = true) {
  const snapshot = useQuery({ ...policyQuery(networkId), enabled })
  const canManage = useCanManage(networkId)
  const data = snapshot.data
  const saved = useMemo(() => (data ? savedPolicy(data) : undefined), [data])
  const draft = usePolicyDraft(networkId, saved)
  const text = draft.value ?? ""
  const document: PolicyDocument | null = useMemo(
    () => (text === "" ? null : readDocument(text)),
    [text]
  )
  const legacy = data?.source_mode === "legacy_v2"
  const savedV3 = data && !legacy ? data.policy : null

  const diagnostics: PlacedDiagnostic[] = useMemo(
    () =>
      (data?.diagnostics ?? []).map((diagnostic) =>
        // A v2 network's diagnostics are about its v2 source: none has a place in v3.
        placeDiagnostic(diagnostic, legacy ? null : savedV3)
      ),
    [data, legacy, savedV3]
  )

  const shown: { policy: NetworkPolicy | null; origin: PolicyOrigin | null } =
    document
      ? { policy: document.policy, origin: draft.dirty ? "draft" : "saved" }
      : legacy && data.migration_proposal
        ? { policy: data.migration_proposal, origin: "proposal" }
        : { policy: null, origin: null }

  /** Sections the draft changes from what is saved. */
  const changed = useMemo(() => {
    const out = new Set<string>()
    if (!draft.dirty || !document?.policy) return out
    for (const section of POLICY_SECTIONS)
      if (!sameData(document.policy[section], savedV3?.[section]))
        out.add(section)
    if (!savedV3) for (const section of POLICY_SECTIONS) out.add(section)
    return out
  }, [draft.dirty, document, savedV3])

  const { edit: editDraft } = draft
  const edit = useCallback(
    (update: (policy: NetworkPolicy) => NetworkPolicy) =>
      editDraft((current) => editPolicy(current, update)),
    [editDraft]
  )
  const move = useCallback(
    (path: JsonPath, index: number, delta: -1 | 1) =>
      editDraft((current) => movePolicy(current, path, index, delta)),
    [editDraft]
  )
  const setText = useCallback(
    (next: string) => editDraft(() => next),
    [editDraft]
  )
  /** Puts a whole policy (the proposal, the template) into the draft as text. */
  const start = useCallback(
    (policy: NetworkPolicy) => editDraft(() => policyText(policy)),
    [editDraft]
  )

  return {
    snapshot,
    data,
    legacy,
    canManage,
    saved,
    draft,
    text,
    document,
    savedPolicy: savedV3,
    diagnostics,
    shown,
    changed,
    editable: canManage && shown.origin !== "proposal" && shown.origin !== null,
    edit,
    move,
    setText,
    start,
  }
}

export type PolicyState = ReturnType<typeof usePolicy>

/** Offers the draft to the commit bar; only the open workspace calls this. */
export function usePolicyCommit(networkId: string, state: PolicyState) {
  const queryClient = useQueryClient()
  const first = state.document?.problems[0]
  return useDraftCommit({
    page: "policy",
    draft: state.draft,
    saved: state.saved,
    problem:
      first && state.document
        ? problemMessage(first, state.document.text)
        : null,
    save: (text, revision) => {
      const policy = readDocument(text).policy
      if (!policy) return Promise.reject(new Error("invalid policy text"))
      return authed((base, token) =>
        saveNetworkPolicy(base, token, networkId, {
          expected_revision: revision,
          policy,
          source_jsonc: text,
        })
      )
    },
    onSaved: (snapshot) => {
      queryClient.setQueryData(policyQuery(networkId).queryKey, snapshot)
      // The policy shares the network's revision.
      void queryClient.invalidateQueries({
        queryKey: networkQuery(networkId).queryKey,
      })
      void queryClient.invalidateQueries({ queryKey: networksQuery().queryKey })
    },
    refresh: () => void state.snapshot.refetch(),
  })
}
