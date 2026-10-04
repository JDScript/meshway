import type { ActivityActor } from "@meshway/control-client"

import { shortId } from "./model"

// Who did something, as the controller recorded it then: a user by the
// email they had at the time (`email`, null when the controller has no
// snapshot), or the controller itself. A missing email is never filled from
// a current address (the account's, or a member list's): the user ID says
// who instead.

export type ActorWords = {
  system: () => string
  /** A user without an email snapshot, by the start of their ID. */
  user: (id: string) => string
  /** This account, under the name it had then. */
  self: (name: string) => string
}

export function actorText(
  actor: ActivityActor,
  selfId: string | null,
  words: ActorWords
): string {
  if (actor.type === "system") return words.system()
  const name = actor.email ?? words.user(shortId(actor.userId))
  return actor.userId === selfId ? words.self(name) : name
}

/** The full user ID behind an actor, for a tooltip; none for the system. */
export const actorId = (actor: ActivityActor) =>
  actor.type === "user" ? actor.userId : undefined
