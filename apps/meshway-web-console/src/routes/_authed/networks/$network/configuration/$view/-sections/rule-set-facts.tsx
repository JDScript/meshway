import type {
  NullableRuleSetProvider,
  RuleSetConversion,
  RuleSetView,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { useQuery } from "@tanstack/react-query"
import { CodeIcon, LibraryIcon, SearchIcon } from "lucide-react"
import { useState } from "react"

import { failureText } from "@/api/failure"
import { ruleSetReportQuery } from "@/api/queries/rule-sets"
import { GlassDialog } from "@/components/chrome/glass-dialog"
import { ListPlaceholders } from "@/components/chrome/list"
import { EmptyLine, Section } from "@/components/chrome/section"
import { NoneLine } from "@/components/configuration/form-kit"
import { BUILTIN_PROVIDERS } from "@/lib/configuration/rule-sets"
import { countText, dateTime } from "@/lib/format"
import { m } from "@/paraglide/messages.js"

import { CatalogDialog } from "./rule-set-browse"

/** A skip reason as people read it (rule-sets.md, 转换); one not known as written. */
export function reasonText(reason: string): string {
  const [head, detail] = reason.split(/:(.*)/s)
  switch (head) {
    case "regex":
      return m.configuration_ruleset_skip_regex()
    case "invalidName":
      return m.configuration_ruleset_skip_invalid_name()
    case "invalidKeyword":
      return m.configuration_ruleset_skip_invalid_keyword()
    case "invalidCidr":
      return m.configuration_ruleset_skip_invalid_cidr()
    case "wildcard":
      return m.configuration_ruleset_skip_wildcard()
    case "mixedRule":
      return m.configuration_ruleset_skip_mixed()
    case "emptyRule":
      return m.configuration_ruleset_skip_empty()
    case "notString":
      return m.configuration_ruleset_skip_not_string()
    case "unsupportedField":
      return m.configuration_ruleset_skip_field({ field: detail ?? "?" })
    case "classical":
      return m.configuration_ruleset_skip_classical({ type: detail ?? "?" })
    default:
      return reason
  }
}

/** What the content holds by kind, the kinds it has. */
function convertedText(conversion: RuleSetConversion): string {
  const { exact, suffix, subdomain, keyword, cidr } = conversion.converted
  return [
    suffix
      ? m.configuration_ruleset_kind_suffix({ count: countText(suffix) })
      : "",
    exact
      ? m.configuration_ruleset_kind_exact({ count: countText(exact) })
      : "",
    subdomain
      ? m.configuration_ruleset_kind_subdomain({ count: countText(subdomain) })
      : "",
    keyword
      ? m.configuration_ruleset_kind_keyword({ count: countText(keyword) })
      : "",
    cidr ? m.configuration_ruleset_kind_cidr({ count: countText(cidr) }) : "",
  ]
    .filter(Boolean)
    .join(" · ")
}

const skippedOf = (conversion: RuleSetConversion) =>
  Object.values(conversion.unconverted).reduce((sum, n) => sum + n, 0)

/**
 * How the bound content was converted from its source: what it holds by
 * kind, what it skipped (each reason opens the entries), and a catalog
 * that holds an older version for now.
 */
export function ConversionFacts({
  networkId,
  name,
  view,
}: {
  networkId: string
  name: string
  view: RuleSetView
}) {
  const [open, setOpen] = useState(false)
  const conversion = view.conversion
  const skipped = conversion ? skippedOf(conversion) : 0
  return (
    <>
      {conversion ? (
        <p className="pl-5.5 text-meta text-muted-foreground tabular-nums">
          {convertedText(conversion) || m.configuration_ruleset_kind_none()}
          {skipped > 0 ? (
            <>
              {" · "}
              <button
                type="button"
                onClick={() => setOpen(true)}
                className="rounded-sm font-medium text-warning underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
              >
                {m.configuration_ruleset_skipped({ count: countText(skipped) })}
              </button>
            </>
          ) : null}
        </p>
      ) : null}
      {view.held ? (
        <p className="pl-5.5 text-meta text-warning">
          {m.configuration_ruleset_held({
            reason: view.held.reason,
            since: dateTime(view.held.since) ?? view.held.since,
          })}
        </p>
      ) : null}
      {conversion && view.artifact && skipped > 0 ? (
        <SkippedDialog
          open={open}
          onOpenChange={setOpen}
          networkId={networkId}
          name={name}
          digest={view.artifact.digest}
          conversion={conversion}
        />
      ) : null}
    </>
  )
}

/** The entries the content skipped: counts by reason, then the first 1,000 with where they were. */
function SkippedDialog({
  open,
  onOpenChange,
  networkId,
  name,
  digest,
  conversion,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  networkId: string
  name: string
  digest: string
  conversion: RuleSetConversion
}) {
  const report = useQuery({
    ...ruleSetReportQuery(networkId, name, digest),
    enabled: open,
  })
  const reasons = Object.entries(conversion.unconverted).sort(
    ([, a], [, b]) => b - a
  )
  const total = skippedOf(conversion)
  const items = report.data?.items
  return (
    <GlassDialog
      open={open}
      onOpenChange={onOpenChange}
      wide
      title={m.configuration_ruleset_skipped_title({ name })}
    >
      <p className="text-row text-muted-foreground">
        {m.configuration_ruleset_skipped_body()}
      </p>
      <ul className="flex flex-wrap gap-1.5">
        {reasons.map(([reason, n]) => (
          <li
            key={reason}
            className="inline-flex h-6 items-center gap-1.5 rounded-full bg-fill-subtle px-2.5 text-meta"
          >
            {reasonText(reason)}
            <span className="text-muted-foreground tabular-nums">
              {countText(n)}
            </span>
          </li>
        ))}
      </ul>
      {items === undefined ? (
        report.isError ? (
          <EmptyLine>
            {failureText(report.error) ??
              m.configuration_ruleset_skipped_failed()}
          </EmptyLine>
        ) : (
          <ul className="flex flex-col gap-0.5">
            <ListPlaceholders rows={4} />
          </ul>
        )
      ) : items.length === 0 ? (
        <EmptyLine>{m.configuration_ruleset_skipped_summary_only()}</EmptyLine>
      ) : (
        <>
          <ol className="flex max-h-[50vh] flex-col overflow-y-auto rounded-tile border border-divider">
            {items.map((item, index) => (
              <li
                key={index}
                className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 border-b border-divider px-3.5 py-2 last:border-b-0"
              >
                <code className="truncate font-mono text-meta">
                  {item.entry}
                </code>
                <span className="text-meta text-muted-foreground">
                  {reasonText(item.reason)}
                </span>
                <span className="col-span-2 font-mono text-[11px] text-muted-foreground">
                  {item.at}
                </span>
              </li>
            ))}
          </ol>
          {items.length < total ? (
            <p className="text-meta text-muted-foreground">
              {m.configuration_ruleset_skipped_first({
                shown: countText(items.length),
                total: countText(total),
              })}
            </p>
          ) : null}
        </>
      )}
    </GlassDialog>
  )
}

/** Where a catalog set came from: its provider, upstream repository, ref, commit and file. */
export function catalogSourceText(view: RuleSetView): string | null {
  const { source, provider } = view
  const parts: string[] = []
  if (provider)
    parts.push(
      m.configuration_ruleset_catalog_from({
        provider: provider.name,
        collection: provider.collection,
      })
    )
  if (source?.repo)
    parts.push(
      [
        source.repo,
        source.ref ? `@${source.ref}` : "",
        source.commit ? ` (${source.commit.slice(0, 7)})` : "",
        source.path ? ` ${source.path}` : "",
      ].join("")
    )
  return parts.length > 0 ? parts.join(" · ") : null
}

type ProviderRow = {
  name: string
  /** Built in, replaced by the configuration, turned off by it, or the configuration's own. */
  kind: "builtin" | "replaced" | "disabled" | "custom"
  provider: NullableRuleSetProvider | undefined
}

/**
 * The catalogs `<provider>:<name>` reads from: the built-in ones and what
 * the configuration replaces, turns off or adds. Edited in the source.
 */
export function ProvidersSection({
  networkId,
  referenced,
  overrides,
  editable,
  onEditSource,
}: {
  networkId: string
  /** The keys the draft's routing names, marked in a catalog browsed. */
  referenced: ReadonlySet<string>
  overrides: Readonly<Record<string, NullableRuleSetProvider>>
  editable: boolean
  onEditSource: () => void
}) {
  const [browsing, setBrowsing] = useState(false)
  const [browsed, setBrowsed] = useState("")
  const rows: ProviderRow[] = [
    ...BUILTIN_PROVIDERS.map((name): ProviderRow => {
      if (!Object.hasOwn(overrides, name))
        return { name, kind: "builtin", provider: undefined }
      const provider = overrides[name]
      return {
        name,
        kind: provider === null ? "disabled" : "replaced",
        provider,
      }
    }),
    ...Object.entries(overrides)
      .filter(([name]) => !BUILTIN_PROVIDERS.includes(name))
      .map(([name, provider]): ProviderRow => ({
        name,
        kind: provider === null ? "disabled" : "custom",
        provider,
      })),
  ]
  const label: Record<ProviderRow["kind"], () => string> = {
    builtin: () => m.configuration_ruleset_provider_builtin(),
    replaced: () => m.configuration_ruleset_provider_replaced(),
    disabled: () => m.configuration_ruleset_provider_disabled(),
    custom: () => m.configuration_ruleset_provider_custom(),
  }
  return (
    <Section
      title={m.configuration_ruleset_providers()}
      count={rows.length}
      action={
        editable ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-7 rounded-full px-3"
            onClick={onEditSource}
          >
            <CodeIcon />
            {m.configuration_ruleset_providers_edit()}
          </Button>
        ) : undefined
      }
    >
      <ul className="flex flex-col divide-y divide-divider overflow-hidden rounded-tile border border-divider">
        {rows.map((row) => (
          <li
            key={row.name}
            className="flex min-h-12 items-start gap-3 px-3.5 py-2.5"
          >
            <LibraryIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="font-mono text-row">{row.name}</span>
              <span className="text-meta break-all text-muted-foreground">
                {row.provider
                  ? [
                      row.provider.catalog,
                      m.configuration_ruleset_provider_collection({
                        collection: row.provider.collection,
                      }),
                      m.configuration_ruleset_provider_keys({
                        count: row.provider.publicKeys.length,
                      }),
                      row.provider.updateInterval
                        ? m.configuration_ruleset_provider_interval({
                            interval: row.provider.updateInterval,
                          })
                        : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")
                  : row.kind === "builtin"
                    ? m.configuration_ruleset_provider_builtin_source()
                    : null}
              </span>
            </span>
            <span className="flex h-5 shrink-0 items-center text-meta text-muted-foreground">
              {label[row.kind]()}
            </span>
            {row.kind !== "disabled" ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="-my-1 h-7 shrink-0 rounded-full px-3"
                onClick={() => {
                  setBrowsed(row.name)
                  setBrowsing(true)
                }}
              >
                <SearchIcon />
                {m.configuration_ruleset_browse()}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
      <CatalogDialog
        open={browsing}
        onOpenChange={setBrowsing}
        networkId={networkId}
        provider={browsed}
        referenced={referenced}
      />
    </Section>
  )
}

/** The catalog sets the routing names, with where each stands; none named says how to. */
export function CatalogSetsEmpty() {
  return <NoneLine>{m.configuration_ruleset_catalog_none()}</NoneLine>
}
