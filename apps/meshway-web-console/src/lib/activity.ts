import type { ActivityAction, ActivityActor } from "@meshway/control-client"

import { shortId } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

// Who did something, as the controller recorded it then: a user by the
// email they had at the time (`email`, null when the controller has no
// snapshot), or the controller itself. A missing email is never filled from
// a current address (the account's, or a member list's): the user ID says
// who instead.

export type ActorWords = {
  system: () => string
  /** A Node acting for itself, by the start of its ID. */
  node: (id: string) => string
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
  if (actor.type === "node") return words.node(shortId(actor.nodeId))
  const name = actor.email ?? words.user(shortId(actor.userId))
  return actor.userId === selfId ? words.self(name) : name
}

/** The full user ID behind an actor, for a tooltip; none for the system. */
export const actorId = (actor: ActivityActor) =>
  actor.type === "user" ? actor.userId : undefined

/** The words for an actor, the same in every activity list. */
export const ACTOR_WORDS: ActorWords = {
  system: () => m.actor_system(),
  node: (id) => m.actor_node({ id }),
  user: (id) => m.actor_user({ id }),
  self: (name) => m.actor_self({ name }),
}

/** What an activity event did, the same in the network's and the account's lists. */
export const activityActionLabel: Record<ActivityAction, () => string> = {
  "account.register": () => m.activity_account_register(),
  "session.login": () => m.activity_session_login(),
  "session.logout": () => m.activity_session_logout(),
  "session.revoke": () => m.activity_session_revoke(),
  "session.revokeOthers": () => m.activity_session_revoke_others(),
  "session.revokeAll": () => m.activity_session_revoke_all(),
  "session.delegate": () => m.activity_session_delegate(),
  "account.password": () => m.activity_account_password(),
  "account.passwordReset": () => m.activity_account_password_reset(),
  "account.resetIssued": () => m.activity_account_reset_issued(),
  "account.email": () => m.activity_account_email(),
  "network.create": () => m.activity_network_create(),
  "network.rename": () => m.activity_network_rename(),
  "configuration.apply": () => m.activity_configuration_apply(),
  "membership.role": () => m.activity_membership_role(),
  "membership.join": () => m.activity_membership_join(),
  "membership.remove": () => m.activity_membership_remove(),
  "joinLink.create": () => m.activity_join_link_create(),
  "joinLink.revoke": () => m.activity_join_link_revoke(),
  "routing.select": () => m.activity_routing_select(),
  "routing.autoResolve": () => m.activity_routing_auto_resolve(),
}
