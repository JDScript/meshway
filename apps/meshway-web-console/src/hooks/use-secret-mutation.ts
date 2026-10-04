import { useMutation } from "@tanstack/react-query"
import { useCallback } from "react"

import { toastFailureWith, type FailureText } from "@/api/failure"

/**
 * Creates something the controller answers with a secret (an enrolment
 * token, a join or reset link). The secret stays in this hook only: never
 * in the URL, the history or the query cache (the mutation is forgotten as
 * soon as nothing shows it), and `clear` drops it when its dialog closes.
 * Not retried: a lost response is not a refusal. `failure` words some
 * problem codes in the call's own terms.
 */
export function useSecretMutation<Variables, Result>(
  create: (variables: Variables) => Promise<Result>,
  failure: FailureText = {}
) {
  const mutation = useMutation({
    mutationFn: create,
    gcTime: 0,
    retry: false,
    onError: toastFailureWith(failure),
  })
  const { reset } = mutation
  return {
    create: mutation.mutate,
    pending: mutation.isPending,
    result: mutation.data ?? null,
    clear: useCallback(() => reset(), [reset]),
  }
}
