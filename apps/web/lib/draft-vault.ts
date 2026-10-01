import { AccessDraftRegistry } from "./access-draft-registry.ts"
import { ConnectionsDraftRegistry } from "./connections-draft-registry.ts"
import { RoutingDraftRegistry } from "./routing-draft-registry.ts"

export type NetworkDrafts = {
  access: AccessDraftRegistry
  connections: ConnectionsDraftRegistry
  routing: RoutingDraftRegistry
}

/**
 * One set of draft registries per network for the signed-in console. A
 * network's registries are created once and returned on every later visit,
 * so its unsaved text, comments, unapplied forms and CAS base come back after
 * switching away; no network can read or clear another's drafts.
 */
export class DraftVault {
  private readonly networks = new Map<string, NetworkDrafts>()

  drafts(networkId: string): NetworkDrafts {
    let drafts = this.networks.get(networkId)
    if (!drafts) {
      drafts = { access: new AccessDraftRegistry(), connections: new ConnectionsDraftRegistry(), routing: new RoutingDraftRegistry() }
      this.networks.set(networkId, drafts)
    }
    return drafts
  }

  /** Whether a network keeps unsaved work. */
  hasDrafts(networkId: string): boolean {
    const drafts = this.networks.get(networkId)
    return drafts !== undefined && (drafts.access.hasDraft() || drafts.connections.hasDraft() || drafts.routing.hasDraft())
  }

  /** Whether any network keeps unsaved work (the page-leave prompt). */
  anyDrafts(): boolean {
    return [...this.networks.keys()].some((networkId) => this.hasDrafts(networkId))
  }

  /** Logout: drops every network's unsaved work. */
  reset() {
    for (const drafts of this.networks.values()) {
      drafts.access.reset()
      drafts.connections.reset()
      drafts.routing.reset()
    }
  }
}
