import type {
  AggregatedPath,
  MembershipRole,
  NodeIdentity,
} from "@meshway/control-client"

// What the start form offers. The controller publishes no list of the paths
// it would allow, so an explicit path is one the network has shown: the
// direct one, those the State reports between the two nodes, and those an
// observation of this tab took. The controller decides; a refusal says so.

const key = (path: readonly string[]) => path.join(">")

export function pathChoices(
  source: string,
  target: string,
  reported: readonly AggregatedPath[],
  observed: readonly (readonly string[])[]
): string[][] {
  const seen = new Set<string>()
  const choices: string[][] = []
  const add = (path: readonly string[]) => {
    if (path[0] !== source || path.at(-1) !== target || path.length < 2) return
    if (new Set(path).size !== path.length) return
    const k = key(path)
    if (seen.has(k)) return
    seen.add(k)
    choices.push([...path])
  }
  add([source, target])
  for (const item of reported) add(item.state.nodes)
  for (const path of observed) add(path)
  return choices.sort(
    (a, b) => a.length - b.length || (key(a) < key(b) ? -1 : 1)
  )
}

/**
 * Whether the controller lets this account start an observation from a
 * node: owners and admins from any node of the network, a member from their
 * own User-identity devices. The controller publishes no action for it, so
 * this restates its rule; it still decides.
 */
export function canObserveFrom(
  role: MembershipRole | null,
  email: string | null,
  identity: NodeIdentity
): boolean {
  if (role === "owner" || role === "admin") return true
  if (role !== "member" || email === null) return false
  return identity.type === "user" && identity.user === email
}
