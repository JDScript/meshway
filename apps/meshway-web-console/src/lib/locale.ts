import { useSyncExternalStore } from "react"

import { getLocale, setLocale, type Locale } from "@/paraglide/runtime.js"

// The language, switched without reloading the page. Paraglide keeps the
// choice (localStorage) and every message reads it when called; the console
// root re-renders its pages under the new one (see main.tsx).

const listeners = new Set<() => void>()

export function switchLocale(locale: Locale) {
  if (locale === getLocale()) return
  void setLocale(locale, { reload: false })
  document.documentElement.lang = locale
  for (const listener of listeners) listener()
}

export function useLocale(): Locale {
  return useSyncExternalStore((listener) => {
    listeners.add(listener)
    return () => listeners.delete(listener)
  }, getLocale)
}
