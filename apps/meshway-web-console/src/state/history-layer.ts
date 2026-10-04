import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import {
  historyWindow,
  type HistoryWindow,
  type WindowChoice,
} from "@/features/history/window"

import { onSessionChange } from "./session"

// The overview's history layer: off by default, and a window chosen on a
// network belongs to that network. The globe draws it on every page of the
// network; the overview offers the choice.

const layerStore = createStore<Readonly<Record<string, HistoryWindow>>>(
  () => ({})
)

onSessionChange(() => layerStore.setState({}, true))

/** Shows the history of the last `choice` (from now), or turns the layer off. */
export function chooseHistoryLayer(
  networkId: string,
  choice: WindowChoice | null
) {
  layerStore.setState((all) => {
    const next = { ...all }
    if (choice) next[networkId] = historyWindow(choice, Date.now())
    else delete next[networkId]
    return next
  }, true)
}

export const useHistoryLayer = (networkId: string): HistoryWindow | null =>
  useStore(layerStore, (all) => all[networkId] ?? null)
