import { defineI18n } from "fumadocs-core/i18n"

/**
 * The site's languages, as the console's: Chinese first, at `/`; English
 * under `/en`. A page `x.mdx` is Chinese, `x.en.mdx` its English.
 */
export const i18n = defineI18n({
  defaultLanguage: "zh",
  languages: ["zh", "en"],
  hideLocale: "default-locale",
  parser: "dot",
})

export type Lang = (typeof i18n.languages)[number]

/** A path on the site in `lang`: the default language has no prefix. */
export const localized = (lang: Lang, path: string) =>
  lang === i18n.defaultLanguage ? path : `/${lang}${path === "/" ? "" : path}`
