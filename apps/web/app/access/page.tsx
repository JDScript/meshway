import { LegacyRedirect } from "@/components/shell/legacy-redirect"

/** Pre-network URL, kept as a redirect into the remembered network. */
export default function LegacyPage({ searchParams }: { searchParams: Promise<Record<string, string | string[] | undefined>> }) {
  return <LegacyRedirect kind="access" searchParams={searchParams} />
}
