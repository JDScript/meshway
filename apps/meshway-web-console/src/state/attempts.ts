import { ApiFailure } from "@/api/client"
import { attemptFor, outcomeUnknown, type Attempt } from "@/lib/attempt"

import { onSessionChange } from "./session"

/** Each kind's unsettled scopes, for the debug panel. */
const kinds = new Map<string, () => readonly string[]>()

/**
 * The unsettled attempts of one `kind` of change the controller
 * deduplicates by request ID, by what each changes (a network, or `""` for
 * none). One whose outcome is unknown stays here past the dialog or page
 * that sent it, so trying the same thing again sends the same request; a
 * sign-in change drops them all. Runtime state: only the debug panel shows
 * it.
 */
export function attempts<Body>(kind: string) {
  const unsettled = new Map<string, Attempt<Body>>()
  onSessionChange(() => unsettled.clear())
  kinds.set(kind, () => [...unsettled.keys()])
  return {
    /**
     * Sends the attempt for `key` (the unsettled one again when it is the
     * same, else a fresh one with `body()`); keeps it only while a failure
     * leaves its outcome unknown.
     */
    async send<Result>(
      scope: string,
      key: string,
      body: () => Body,
      run: (attempt: Attempt<Body>) => Promise<Result>
    ): Promise<Result> {
      const attempt = attemptFor(unsettled.get(scope) ?? null, key, body)
      try {
        const result = await run(attempt)
        unsettled.delete(scope)
        return result
      } catch (error) {
        if (error instanceof ApiFailure && outcomeUnknown(error.failure))
          unsettled.set(scope, attempt)
        else unsettled.delete(scope)
        throw error
      }
    },
    /** Whether `scope`'s last attempt may have happened (its outcome is unknown). */
    unknown: (scope: string) => unsettled.has(scope),
  }
}

/** The kinds with attempts whose outcome is unknown, and their scopes. */
export const unsettledAttempts = (): { kind: string; scopes: string[] }[] =>
  [...kinds]
    .map(([kind, scopes]) => ({ kind, scopes: [...scopes()] }))
    .filter((entry) => entry.scopes.length > 0)
