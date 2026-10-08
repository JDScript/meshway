import { DocsLayout } from "fumadocs-ui/layouts/docs"
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
} from "fumadocs-ui/layouts/docs/page"
import { createRelativeLink } from "fumadocs-ui/mdx"
import type { Metadata } from "next"
import { notFound } from "next/navigation"
import type { ReactNode } from "react"

import { getMDXComponents } from "@/components/mdx"
import { baseOptions } from "@/components/site/layout"
import type { Lang } from "@/lib/i18n"
import { source } from "@/lib/source"

/** The docs of one language: the sidebar from its pages, then the page. */
export function DocsShell({
  lang,
  children,
}: {
  lang: Lang
  children: ReactNode
}) {
  return (
    <DocsLayout tree={source.getPageTree(lang)} {...baseOptions(lang)}>
      {children}
    </DocsLayout>
  )
}

export function DocsView({ lang, slug }: { lang: Lang; slug?: string[] }) {
  const page = source.getPage(slug, lang)
  if (!page) notFound()
  const MDX = page.data.body
  return (
    <DocsPage toc={page.data.toc} full={page.data.full}>
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <DocsBody>
        <MDX
          components={getMDXComponents({ a: createRelativeLink(source, page) })}
        />
      </DocsBody>
    </DocsPage>
  )
}

/** The pages of `lang`, as its route's params. */
export const docsParams = (lang: Lang) =>
  source
    .generateParams()
    .filter((params) => params.lang === lang)
    .map(({ slug }) => ({ slug }))

export function docsMetadata(lang: Lang, slug?: string[]): Metadata {
  const page = source.getPage(slug, lang)
  if (!page) notFound()
  return { title: page.data.title, description: page.data.description }
}
