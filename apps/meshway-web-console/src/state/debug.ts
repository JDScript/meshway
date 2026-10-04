import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

// The debug mode: whether this browser has it on (localStorage, shared by
// its tabs), and whether this tab shows its panel (sessionStorage, kept
// across a reload). While it is on, the console records what its streams,
// requests and cache do (`debug-trace.ts`); while it is off nothing is
// recorded and the panel's code is not loaded. Ctrl+` shows or hides the
// panel while the mode is on.

const MODE_KEY = "meshway.debug"
const PANEL_KEY = "meshway.debug.panel"

export type DebugTab =
  "stream" | "consistency" | "cache" | "requests" | "local" | "time"

type Debug = {
  enabled: boolean
  open: boolean
  tab: DebugTab
  /** The panel's height in pixels, as last dragged in this tab. */
  height: number
}

const DEFAULT_HEIGHT = 360
const MIN_HEIGHT = 200
/** What the panel leaves of the window above it. */
const TOP_CLEARANCE = 120

function stored(storage: () => Storage, key: string): string | null {
  try {
    return storage().getItem(key)
  } catch {
    return null
  }
}

function keep(storage: () => Storage, key: string, value: string | null) {
  try {
    if (value === null) storage().removeItem(key)
    else storage().setItem(key, value)
  } catch {
    // A preference only.
  }
}

const local = () => window.localStorage
const session = () => window.sessionStorage

const debugStore = createStore<Debug>(() => ({
  enabled: stored(local, MODE_KEY) === "on",
  open: stored(session, PANEL_KEY) !== "closed",
  tab: "stream",
  height: DEFAULT_HEIGHT,
}))

/** Whether the mode is on now, outside React (what records asks). */
export const debugEnabled = (): boolean => debugStore.getState().enabled

/** Calls `listener` when the mode turns on or off: recording starts afresh or drops. */
export const onDebugToggle = (listener: () => void): (() => void) =>
  debugStore.subscribe((state, previous) => {
    if (state.enabled !== previous.enabled) listener()
  })

function setOpen(open: boolean) {
  keep(session, PANEL_KEY, open ? null : "closed")
  debugStore.setState({ open })
}

/** Turns the mode on (showing the panel) or off. */
export function setDebugEnabled(enabled: boolean) {
  keep(local, MODE_KEY, enabled ? "on" : null)
  debugStore.setState({ enabled })
  if (enabled) setOpen(true)
}

export const closeDebugPanel = () => setOpen(false)

export const setDebugTab = (tab: DebugTab) => debugStore.setState({ tab })

/** The panel's height, kept between its minimum and what leaves the window's top clear. */
export function setDebugHeight(height: number) {
  const ceiling = Math.max(MIN_HEIGHT, window.innerHeight - TOP_CLEARANCE)
  debugStore.setState({
    height: Math.round(Math.min(ceiling, Math.max(MIN_HEIGHT, height))),
  })
}

export const useDebugEnabled = (): boolean =>
  useStore(debugStore, (state) => state.enabled)

/** Whether this tab shows the panel: the mode is on and the panel open. */
export const useDebugPanelShown = (): boolean =>
  useStore(debugStore, (state) => state.enabled && state.open)

export const useDebugTab = (): DebugTab =>
  useStore(debugStore, (state) => state.tab)

export const useDebugHeight = (): number =>
  useStore(debugStore, (state) => state.height)

/**
 * Keeps the mode in step with the other tabs and lets Ctrl+` show or hide
 * the panel while it is on. Called once, by `main.tsx`.
 */
export function watchDebugMode() {
  window.addEventListener("storage", (event) => {
    if (event.storageArea !== window.localStorage) return
    if (event.key !== MODE_KEY && event.key !== null) return
    debugStore.setState({ enabled: event.newValue === "on" })
  })
  window.addEventListener("keydown", (event) => {
    if (event.repeat || !event.ctrlKey || event.code !== "Backquote") return
    const { enabled, open } = debugStore.getState()
    if (!enabled) return
    event.preventDefault()
    setOpen(!open)
  })
}
