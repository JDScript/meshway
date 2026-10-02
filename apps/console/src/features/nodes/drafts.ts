import type { TagDefinition } from "@meshway/control-client"

import { createDraft, type Draft, type Saved } from "@/features/shell/drafts"

import type { ListenerFields } from "./listener-model"

// The nodes page's drafts. Each editable resource has its own: one per
// listener (and one for a node's listener being added), one per node's
// tags, and one for the tag catalog. All are saved against the network's
// revision, which is what the controller checks.

type UseDraft<T> = (networkId: string, saved: Saved<T> | undefined) => Draft<T>

function family<T>(): (key: string) => UseDraft<T> {
  const hooks = new Map<string, UseDraft<T>>()
  return (key) => {
    let hook = hooks.get(key)
    if (!hook) {
      hook = createDraft<T>("nodes")
      hooks.set(key, hook)
    }
    return hook
  }
}

/** A listener's draft, by `<node>/<listener>` (the editor is keyed by it, so the hook is fixed per instance). */
export const listenerDraft = family<ListenerFields>()

/** A node's tag assignment draft, by node. */
export const nodeTagsDraft = family<string[]>()

export const useTagCatalogDraft = createDraft<TagDefinition[]>("nodes")
