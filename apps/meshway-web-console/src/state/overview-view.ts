import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

export type OverviewView = "globe" | "topology"
const KEY = "meshway.overview-view"
const store = createStore<{ view: OverviewView }>(() => {
  try {
    return {
      view: localStorage.getItem(KEY) === "topology" ? "topology" : "globe",
    }
  } catch {
    return { view: "globe" }
  }
})
export const useOverviewView = () => useStore(store, (state) => state.view)
export function chooseOverviewView(view: OverviewView) {
  store.setState({ view })
  try {
    localStorage.setItem(KEY, view)
  } catch {
    /* The in-memory preference still works. */
  }
}
