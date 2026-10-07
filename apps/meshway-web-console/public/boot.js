// Runs before the app (a same-origin file, so a strict CSP allows it):
// applies the stored theme at once, so the first paint is never the wrong colour.
;(function () {
  // While the sun is followed (the default), the theme last shown: the app
  // knows where the sun is and corrects it, under the loading screen.
  var stored = null
  var shown = null
  try {
    stored = localStorage.getItem("theme")
    shown = localStorage.getItem("theme-shown")
  } catch (error) {}
  var system = window.matchMedia("(prefers-color-scheme: dark)").matches
  var sun = stored !== "dark" && stored !== "light" && stored !== "system"
  var dark =
    stored === "dark" ||
    (stored === "system" && system) ||
    (sun && (shown ? shown === "dark" : system))
  document.documentElement.classList.add(dark ? "dark" : "light")
})()
