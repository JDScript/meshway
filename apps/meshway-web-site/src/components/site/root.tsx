import { i18nProvider } from "fumadocs-ui/i18n"
import type { ReactNode } from "react"

import { Provider } from "@/components/provider"
import type { Lang } from "@/lib/i18n"
import { translations } from "@/lib/translations"

import "../../app/global.css"

/** The document of one language: each language is its own root layout. */
export function SiteRoot({
  lang,
  children,
}: {
  lang: Lang
  children: ReactNode
}) {
  return (
    <html lang={lang === "zh" ? "zh-CN" : "en"} suppressHydrationWarning>
      <body className="flex min-h-screen flex-col">
        <Provider i18n={i18nProvider(translations, lang)}>{children}</Provider>
      </body>
    </html>
  )
}
