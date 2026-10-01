import { LegacyRedirect } from "@/components/shell/legacy-redirect"

/** Pre-network node URL, kept as a redirect into the remembered network. */
export default function LegacyNodePage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>
  searchParams: Promise<Record<string, string | string[] | undefined>>
}) {
  return <LegacyRedirect kind="node" nodeId={params} searchParams={searchParams} />
}
