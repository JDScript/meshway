import { BrandMark } from "@meshway/ui/brand"
import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared"

import { localized, type Lang } from "@/lib/i18n"
import { appName, consoleUrl } from "@/lib/site"

const words = {
  zh: { docs: "文档", signIn: "登录 console" },
  en: { docs: "Docs", signIn: "Sign in to the console" },
} satisfies Record<Lang, Record<string, string>>

/** What every page's header carries: the brand, the docs and the console. */
export function baseOptions(lang: Lang): BaseLayoutProps {
  return {
    i18n: true,
    nav: {
      title: (
        <span className="inline-flex items-center gap-2 font-semibold tracking-tight">
          <BrandMark className="size-5" />
          {appName}
        </span>
      ),
      url: localized(lang, "/"),
    },
    links: [
      {
        text: words[lang].docs,
        url: localized(lang, "/docs"),
        active: "nested-url",
      },
      { text: words[lang].signIn, url: consoleUrl, external: true },
    ],
  }
}
