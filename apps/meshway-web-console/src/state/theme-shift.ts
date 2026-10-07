// A change of theme as a sunrise or a sunset (index.css, "Theme shift"):
// the new theme rises from the bottom like daylight (to light) or falls
// from the top like night (to dark), a band of dawn or dusk light along its
// edge, while the globe's light settles (globe/engine.ts). Where the
// browser cannot (no View Transitions) or the reader asked for less
// motion, the colours change at once.

const MOTION = "(prefers-reduced-motion: reduce)"

/** As index.css's `--theme-shift-ms`. */
export const THEME_SHIFT_MS = 1600

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

/** Applies a theme (`apply` swaps the class); `animate` makes it a sunrise or a sunset. */
export function shiftTheme(
  toDark: boolean,
  animate: boolean,
  apply: () => void
) {
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
  // A shift under way jumps to its end; the new one starts from there.
  running?.skipTransition()
  const shift = toDark ? "sunset" : "sunrise"
  const band = document.createElement("div")
  band.className = "theme-shift-band"
  band.dataset.shift = shift
  band.setAttribute("aria-hidden", "true")
  const transition = document.startViewTransition(() => {
    root.dataset.themeShift = shift
    withoutTransitions(apply)
    document.body.append(band)
  })
  running = transition
  // The band moves with the edge, started as the edge is.
  transition.ready.then(
    () => band.setAttribute("data-running", ""),
    () => undefined
  )
  void transition.finished.finally(() => {
    band.remove()
    if (running === transition) {
      running = null
      delete root.dataset.themeShift
    }
  })
}
