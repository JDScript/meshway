import { docsMetadata, docsParams, DocsView } from "@/components/site/docs"

export default async function Page(props: PageProps<"/docs/[[...slug]]">) {
  const { slug } = await props.params
  return <DocsView lang="zh" slug={slug} />
}

export const generateStaticParams = () => docsParams("zh")

export async function generateMetadata(props: PageProps<"/docs/[[...slug]]">) {
  const { slug } = await props.params
  return docsMetadata("zh", slug)
}
