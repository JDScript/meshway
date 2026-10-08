import { docsMetadata, docsParams, DocsView } from "@/components/site/docs"

export default async function Page(props: PageProps<"/en/docs/[[...slug]]">) {
  const { slug } = await props.params
  return <DocsView lang="en" slug={slug} />
}

export const generateStaticParams = () => docsParams("en")

export async function generateMetadata(
  props: PageProps<"/en/docs/[[...slug]]">
) {
  const { slug } = await props.params
  return docsMetadata("en", slug)
}
