import type { TagDefinition } from "@meshway/control-client"

import { createDraft, createKeyedDraft } from "@/features/shell/drafts"

import type { ListenerFields } from "./listener-model"

// The nodes page's drafts. Each editable resource has its own: one per
// listener (and one for a node's listener being added), one per node's
// tags, and one for the tag catalog. All are saved against the network's
// revision, which is what the controller checks.

/** A listener's draft, keyed `<node>/<listener>`. */
export const useListenerDraft = createKeyedDraft<ListenerFields>("nodes")

/** A node's tag assignment draft, keyed by node. */
export const useNodeTagsDraft = createKeyedDraft<string[]>("nodes")

export const useTagCatalogDraft = createDraft<TagDefinition[]>("nodes")
