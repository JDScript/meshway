import { useMutation } from "@tanstack/react-query"
import { toast } from "sonner"

import { failureOf, toastFailure } from "@/lib/failure"
import { m } from "@/paraglide/messages.js"

import { useCommit } from "./commit-bar"
import type { Draft, Saved } from "./drafts"
import type { PageId } from "./pages"

/**
 * Connects an editor's draft to the commit bar: saving sends the draft
 * against the revision it was loaded at; success hands the controller's
 * answer to `onSaved` (put it in the cache) and ends the draft; a refusal
 * keeps the draft and says why. On a conflict, `refresh` re-reads the
 * resource so the bar can offer to overwrite the newer revision.
 */
export function useDraftCommit<T, R>({
  page,
  draft,
  saved,
  problem,
  save,
  onSaved,
  refresh,
}: {
  page: PageId
  draft: Draft<T>
  saved: Saved<T> | undefined
  /** What stops saving, or null. */
  problem: string | null
  save: (value: T, revision: number) => Promise<R>
  onSaved: (result: R) => void
  refresh: () => void
}) {
  const mutation = useMutation({
    mutationFn: ({ value, revision }: { value: T; revision: number }) =>
      save(value, revision),
    onSuccess: (result) => {
      onSaved(result)
      draft.discard()
      toast.success(m.commit_saved())
    },
    onError: (error) => {
      toastFailure(error)
      if (failureOf(error) === "conflict") refresh()
    },
  })

  const value = draft.value
  const revision = draft.baseRevision
  const submit = (against: number | undefined) => {
    if (
      value === undefined ||
      against === undefined ||
      problem !== null ||
      mutation.isPending
    )
      return
    mutation.mutate({ value, revision: against })
  }

  useCommit(
    draft.dirty
      ? {
          page,
          problem,
          outdated: draft.outdated,
          saving: mutation.isPending,
          save: () => submit(revision),
          discard: draft.discard,
          overwrite: () => submit(saved?.revision),
        }
      : null
  )

  /** For the editor's own form: Enter saves too. */
  return {
    submit: () =>
      draft.dirty && !draft.outdated ? submit(revision) : undefined,
    saving: mutation.isPending,
  }
}
