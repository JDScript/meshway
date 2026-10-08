import { BrandMark } from "@meshway/ui/brand"
import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared"

import { localized, type Lang } from "@/lib/i18n"
import { appName, consoleUrl } from "@/lib/site"

const words = {
  zh: { docs: "文档", api: "API 参考", signIn: "登录控制台" },
  en: { docs: "Docs", api: "API reference", signIn: "Sign in to the console" },
} satisfies Record<Lang, Record<string, string>>

const title = (
  <span className="inline-flex items-center gap-2 font-semibold tracking-tight">
    <BrandMark className="size-5" />
    {appName}
  </span>
)

const signIn = (lang: Lang) => ({
  text: words[lang].signIn,
  url: consoleUrl,
  external: true,
})

/** The landing's header: the brand, the docs, the API reference and the console. */
export function homeOptions(lang: Lang): BaseLayoutProps {
  return {
    i18n: true,
    nav: { title, url: localized(lang, "/") },
    links: [
      { text: words[lang].docs, url: localized(lang, "/docs") },
      { text: words[lang].api, url: localized(lang, "/docs/api") },
      signIn(lang),
    ],
  }
}

/** The docs' sidebar: the brand and the console; its tabs switch between the guides and the API reference. */
export function docsOptions(lang: Lang): BaseLayoutProps {
  return {
    i18n: true,
    nav: { title, url: localized(lang, "/") },
    links: [signIn(lang)],
  }
}
