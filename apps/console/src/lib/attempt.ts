import type { ClientFailure } from "@meshway/control-client"

// A change the controller deduplicates by `requestId` (creating a network,
// renaming it). Each distinct attempt gets a fresh ID; trying the same thing
// again after an attempt whose outcome is unknown sends the same request
// again, ID and body, so the controller answers it once rather than twice.

export type Attempt<Body> = {
  requestId: string
  /** What makes two attempts the same, as the person sees it (the name typed). */
  key: string
  body: Body
}

/**
 * The attempt to send for `key`: the unsettled one again when it is the
 * same, else a new one with a fresh ID and `body()` built now.
 */
export function attemptFor<Body>(
  unsettled: Attempt<Body> | null,
  key: string,
  body: () => Body,
  fresh: () => string = () => crypto.randomUUID()
): Attempt<Body> {
  if (unsettled && unsettled.key === key) return unsettled
  return { requestId: fresh(), key, body: body() }
}

/**
 * Whether a failed attempt may still have happened: only a controller's own
 * refusal (a 4xx) says it did not. A lost or unreadable response, a busy or
 * failing controller, or a gateway answering for it leaves it unknown.
 */
export function outcomeUnknown(failure: ClientFailure<unknown>): boolean {
  return !(failure.kind === "http" && failure.status < 500)
}
