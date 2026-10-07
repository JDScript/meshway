// A change of theme: the globe's sun goes round to the other side
// (globe/engine.ts), so the line between day and night sweeps across it,
// while the interface's colours cross-fade (one View Transition, the globe
// left out of it, index.css "Theme shift"). Where the browser cannot (no
// View Transitions) or the reader asked for less motion, the interface
// changes at once; the globe turns or jumps by the same preference.

const MOTION = "(prefers-reduced-motion: reduce)"

/** How long the globe's sun takes to go round. */
export const THEME_SHIFT_MS = 2400

/** Turns transitions off for the frame a theme changes in, so colours switch at once. */
function withoutTransitions(change: () => void) {
  const style = document.createElement("style")
  style.textContent =
    "*,*::before,*::after{-webkit-transition:none!important;transition:none!important}"
  document.head.appendChild(style)
  change()
  window.getComputedStyle(document.body)
  requestAnimationFrame(() => requestAnimationFrame(() => style.remove()))
}

let running: ViewTransition | null = null

/** Applies a theme (`apply` swaps the class); `animate` cross-fades the interface. */
export function shiftTheme(animate: boolean, apply: () => void) {
  const root = document.documentElement
  if (
    !animate ||
    typeof document.startViewTransition !== "function" ||
    window.matchMedia(MOTION).matches ||
    document.visibilityState !== "visible"
  ) {
    withoutTransitions(apply)
    return
  }
  // A cross-fade under way jumps to its end; the new one starts from there.
  running?.skipTransition()
  // Set before the old state is captured: it names the layers.
  root.dataset.themeShift = ""
  const transition = document.startViewTransition(() =>
    withoutTransitions(apply)
  )
  running = transition
  void transition.finished.finally(() => {
    if (running !== transition) return
    running = null
    delete root.dataset.themeShift
  })
}
