import type { ActivityEvent } from "@meshway/control-client"

import { shortId } from "@/lib/format"

// What the account page reads out of the controller's answers, kept free of
// React and the messages so it can be tested on its own.

/** Which sessions a revocation ends. */
export type Revocation =
  { kind: "one"; sessionId: string } | { kind: "others" } | { kind: "all" }

/** Whether a revocation also ends the session this tab holds. */
export function endsThisTab(revocation: Revocation, thisSession: string) {
  switch (revocation.kind) {
    case "one":
      return revocation.sessionId === thisSession
    case "others":
      return false
    case "all":
      return true
  }
}

/** The start of an ID, enough to tell sessions apart in a list. */

/** What an activity event acted on, as the account page names it. */
export type ActivityTargetView =
  | { kind: "account" }
  | { kind: "this_session" }
  | { kind: "session"; short: string }
  | { kind: "network"; short: string }
  | { kind: "membership"; short: string }
  | { kind: "join_link"; short: string }
  | { kind: "configuration"; revision: number }

export function activityTarget(
  target: ActivityEvent["target"],
  thisSession: string | null
): ActivityTargetView {
  switch (target.type) {
    case "account":
      return { kind: "account" }
    case "session":
      return target.sessionId === thisSession
        ? { kind: "this_session" }
        : { kind: "session", short: shortId(target.sessionId) }
    case "network":
      return { kind: "network", short: shortId(target.networkId) }
    case "membership":
      return { kind: "membership", short: shortId(target.membershipId) }
    case "joinLink":
      return { kind: "join_link", short: shortId(target.linkId) }
    case "configuration":
      return { kind: "configuration", revision: target.revision }
  }
}

/** How many rows a paged list holds: `12+` while the controller has more. */
export const pagedCount = (rows: number, more: boolean) =>
  more ? `${rows}+` : rows
