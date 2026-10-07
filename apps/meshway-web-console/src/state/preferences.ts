import { useStore } from "zustand"
import { createStore } from "zustand/vanilla"

import {
  getLocale,
  setLocale as keepLocale,
  type Locale,
} from "@/paraglide/runtime.js"

import { shiftTheme } from "./theme-shift"

// This browser's preferences: the theme (kept in localStorage, shared by
// its tabs) and the language (Paraglide keeps it; every message reads it
// when called, and the console root renders its pages afresh under a new
// one, see main.tsx). `applyPreferences` puts them on the document.

/** The system's scheme, or light or dark as chosen. */
export type Theme = "system" | "light" | "dark"

export const THEMES: readonly Theme[] = ["system", "light", "dark"]

const THEME_KEY = "theme"
const DARK_SCHEME = "(prefers-color-scheme: dark)"

export const isTheme = (value: unknown): value is Theme =>
  THEMES.some((theme) => theme === value)

function storedTheme(): Theme {
  try {
    const value = window.localStorage.getItem(THEME_KEY)
    return isTheme(value) ? value : "system"
  } catch {
    return "system"
  }
}

type Preferences = {
  theme: Theme
  /** The system's colour scheme, followed while the theme is `system`. */
  systemDark: boolean
  locale: Locale
}

const preferencesStore = createStore<Preferences>(() => ({
  theme: storedTheme(),
  systemDark: window.matchMedia(DARK_SCHEME).matches,
  locale: getLocale(),
}))

const isDark = ({ theme, systemDark }: Preferences) =>
  theme === "dark" || (theme === "system" && systemDark)

export function setTheme(theme: Theme) {
  try {
    window.localStorage.setItem(THEME_KEY, theme)
  } catch {
    // A preference only.
  }
  preferencesStore.setState({ theme })
}

/** The other one of dark and light, from whatever shows now. */
export const toggleTheme = () =>
  setTheme(isDark(preferencesStore.getState()) ? "light" : "dark")

export function switchLocale(locale: Locale) {
  if (locale === preferencesStore.getState().locale) return
  void keepLocale(locale, { reload: false })
  preferencesStore.setState({ locale })
}

export const useTheme = (): Theme =>
  useStore(preferencesStore, (preferences) => preferences.theme)

/** Whether the console shows dark now. */
export const useDark = (): boolean => useStore(preferencesStore, isDark)

export const useLocale = (): Locale =>
  useStore(preferencesStore, (preferences) => preferences.locale)

/** An edit field has the key: a shortcut letter is typing there. */
function typing(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return (
    target.isContentEditable ||
    target.closest("input, textarea, select, [contenteditable='true']") !== null
  )
}

/**
 * Puts the preferences on the document and keeps them there: the theme's
 * class and the language on `<html>`, the system's scheme followed, another
 * tab's theme taken up, and D (outside a field) toggling dark and light. A
 * change after the first turns the globe's light (state/theme-shift.ts).
 * Called once, by `main.tsx`.
 */
export function applyPreferences() {
  const root = document.documentElement
  let first = true
  const show = (preferences: Preferences) => {
    const dark = isDark(preferences)
    if (root.classList.contains(dark ? "dark" : "light")) return
    shiftTheme(!first, () => {
      root.classList.remove("light", "dark")
      root.classList.add(dark ? "dark" : "light")
    })
  }
  show(preferencesStore.getState())
  first = false
  root.lang = preferencesStore.getState().locale
  preferencesStore.subscribe((preferences) => {
    show(preferences)
    root.lang = preferences.locale
  })

  window
    .matchMedia(DARK_SCHEME)
    .addEventListener("change", (event) =>
      preferencesStore.setState({ systemDark: event.matches })
    )
  window.addEventListener("storage", (event) => {
    if (event.storageArea !== window.localStorage || event.key !== THEME_KEY)
      return
    preferencesStore.setState({
      theme: isTheme(event.newValue) ? event.newValue : "system",
    })
  })
  window.addEventListener("keydown", (event) => {
    if (event.repeat || event.metaKey || event.ctrlKey || event.altKey) return
    if (event.key.toLowerCase() !== "d" || typing(event.target)) return
    toggleTheme()
  })
}
