import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import {
  secretFromFragment,
  type FragmentKind,
} from "@/features/entry/fragment"

// The one-time secret of the join or reset link this tab opened, in memory
// only. A page's route takes it from the fragment and replaces the address
// without it; the page reads it from here, across a sign-in on its card.

type Held = {
  /** `undefined` before any fragment, `null` for one without a secret. */
  secret: string | null | undefined
  /** Fragments taken so far: a second link changes only the fragment, and the page starts over. */
  taken: number
}

const NONE: Held = { secret: undefined, taken: 0 }

const secretStore = createStore<Readonly<Record<FragmentKind, Held>>>(() => ({
  join: NONE,
  reset: NONE,
}))

/** Reads `hash` into memory for `kind`'s page. */
export function takeLinkSecret(kind: FragmentKind, hash: string) {
  secretStore.setState((all) => ({
    [kind]: { secret: secretFromFragment(hash), taken: all[kind].taken + 1 },
  }))
}

/** After it has been used, or the person leaves it behind. */
export function dropLinkSecret(kind: FragmentKind) {
  secretStore.setState((all) => ({
    [kind]: { secret: undefined, taken: all[kind].taken },
  }))
}

export const linkSecret = (kind: FragmentKind): string | null | undefined =>
  secretStore.getState()[kind].secret

export const useLinkTaken = (kind: FragmentKind): number =>
  useStore(secretStore, (all) => all[kind].taken)
