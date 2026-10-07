// Runs before the app (a same-origin file, so a strict CSP allows it):
// applies the stored theme at once, so the first paint is never the wrong colour.
;(function () {
  var stored = null
  try {
    stored = localStorage.getItem("theme")
  } catch (error) {}
  var dark = stored === "dark" || (stored !== "light" && window.matchMedia("(prefers-color-scheme: dark)").matches)
  document.documentElement.classList.add(dark ? "dark" : "light")
  try {
    if (localStorage.getItem("look") === "planet")
      document.documentElement.classList.add("planet")
  } catch (error) {}
})()
