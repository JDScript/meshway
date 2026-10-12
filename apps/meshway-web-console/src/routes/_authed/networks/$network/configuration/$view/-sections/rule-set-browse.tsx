import type {
  RuleSetEntryKind,
  RuleSetMember,
  RuleSetMemberCounts,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { useInfiniteQuery, useQuery } from "@tanstack/react-query"
import { CircleCheckIcon, CircleMinusIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import { failureText } from "@/api/failure"
import {
  ruleSetCatalogQuery,
  ruleSetEntriesQuery,
  ruleSetMatchQuery,
} from "@/api/queries/rule-sets"
import { FieldInput } from "@/components/chrome/form"
import { GlassDialog } from "@/components/chrome/glass-dialog"
import { ListPlaceholders } from "@/components/chrome/list"
import { EmptyLine } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { Spinner } from "@/components/ui/spinner"
import { useLoadMore } from "@/hooks/use-load-more"
import { useSettled } from "@/hooks/use-settled"
import { countText, dateTime } from "@/lib/format"
import { isIp, rangeCidr } from "@/lib/ip"
import { m } from "@/paraglide/messages.js"

const KINDS: readonly RuleSetEntryKind[] = ["domain", "keyword", "ipv4", "ipv6"]

const KIND_LABEL: Record<RuleSetEntryKind, () => string> = {
  domain: () => m.configuration_ruleset_members_domain(),
  keyword: () => m.configuration_ruleset_members_keyword(),
  ipv4: () => m.configuration_ruleset_members_ipv4(),
  ipv6: () => m.configuration_ruleset_members_ipv6(),
}

const KIND_SEARCH: Record<RuleSetEntryKind, () => string> = {
  domain: () => m.configuration_ruleset_members_search_domain(),
  keyword: () => m.configuration_ruleset_members_search_keyword(),
  ipv4: () => m.configuration_ruleset_members_search_ipv4(),
  ipv6: () => m.configuration_ruleset_members_search_ipv6(),
}

const countOf = (counts: RuleSetMemberCounts, kind: RuleSetEntryKind) =>
  kind === "domain"
    ? counts.exact + counts.suffix + counts.subdomain
    : counts[kind]

/** A member as the rule-set form writes it, with what kind it is. */
function memberParts(member: RuleSetMember): { text: string; kind: string } {
  switch (member.kind) {
    case "suffix":
      return {
        text: member.name,
        kind: m.configuration_ruleset_member_suffix(),
      }
    case "exact":
      return {
        text: member.name,
        kind: m.configuration_ruleset_member_exact(),
      }
    case "subdomain":
      return {
        text: `.${member.name}`,
        kind: m.configuration_ruleset_member_subdomain(),
      }
    case "keyword":
      return {
        text: member.keyword,
        kind: m.configuration_ruleset_member_keyword(),
      }
    case "ipRange":
      return {
        text:
          rangeCidr(member.first, member.last) ??
          `${member.first} – ${member.last}`,
        kind: m.configuration_ruleset_member_range(),
      }
  }
}

/**
 * What a set's bound content holds: whether it holds a name or an address
 * (as the nodes judge it), then its members one kind at a time, searched
 * the controller's way. Read by digest: the same content lists the same.
 */
export function ContentsDialog({
  open,
  onOpenChange,
  networkId,
  name,
  digest,
  first,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  networkId: string
  name: string
  digest: string
  /** The kind shown first (the content's commonest, as far as it is known). */
  first: RuleSetEntryKind
}) {
  return (
    <GlassDialog
      open={open}
      onOpenChange={onOpenChange}
      wide
      title={m.configuration_ruleset_contents_title({ name })}
    >
      {open ? (
        <Contents
          networkId={networkId}
          name={name}
          digest={digest}
          first={first}
        />
      ) : null}
    </GlassDialog>
  )
}

function Contents({
  networkId,
  name,
  digest,
  first,
}: {
  networkId: string
  name: string
  digest: string
  first: RuleSetEntryKind
}) {
  const [probe, setProbe] = useState("")
  const [kind, setKind] = useState<RuleSetEntryKind>(first)
  const [search, setSearch] = useState("")
  const [counts, setCounts] = useState<RuleSetMemberCounts | null>(null)
  const q = useSettled(search.trim())
  const entries = useInfiniteQuery(
    ruleSetEntriesQuery(networkId, name, digest, kind, q)
  )
  const loadMore = useLoadMore(entries)
  // Every page says how many of each kind the content has.
  const read = entries.data?.pages[0]?.counts
  if (read && counts === null) setCounts(read)
  const items = entries.data?.pages.flatMap((page) => page.items)

  let list: ReactNode
  if (items === undefined)
    list = entries.isError ? (
      <EmptyLine>
        {failureText(entries.error) ?? m.configuration_ruleset_members_failed()}
      </EmptyLine>
    ) : (
      <ul className="flex flex-col">
        <ListPlaceholders rows={6} />
      </ul>
    )
  else if (items.length === 0)
    list = (
      <EmptyLine>
        {q === ""
          ? m.configuration_ruleset_members_none()
          : m.configuration_ruleset_members_no_match()}
      </EmptyLine>
    )
  else
    list = (
      <ol className="flex max-h-[45vh] flex-col overflow-y-auto overscroll-contain rounded-tile border border-divider">
        {items.map((member, index) => {
          const { text, kind: label } = memberParts(member)
          return (
            <li
              key={index}
              className="flex h-9 shrink-0 items-center gap-3 border-b border-divider px-3.5 last:border-b-0"
            >
              <code className="min-w-0 flex-1 truncate font-mono text-meta">
                {text}
              </code>
              <span className="shrink-0 text-meta text-muted-foreground">
                {label}
              </span>
            </li>
          )
        })}
      </ol>
    )

  return (
    <div className="flex flex-col gap-4">
      <MatchProbe
        networkId={networkId}
        name={name}
        digest={digest}
        value={probe}
        onChange={setProbe}
      />
      <div className="flex flex-col gap-2">
        <SegmentedControl
          label={m.configuration_ruleset_members_kind()}
          size="sm"
          value={kind}
          options={KINDS.map((option) => ({
            value: option,
            label: (
              <span className="tabular-nums">
                {KIND_LABEL[option]()}
                {counts ? (
                  <span className="ml-1.5 text-muted-foreground">
                    {countText(countOf(counts, option))}
                  </span>
                ) : null}
              </span>
            ),
          }))}
          onChange={(next) => {
            setKind(next)
            setSearch("")
          }}
          className="self-start"
        />
        <FieldInput
          type="search"
          name="rule-set-member-search"
          aria-label={m.configuration_ruleset_members_search()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={253}
          placeholder={KIND_SEARCH[kind]()}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          className="h-9 font-mono md:text-meta"
        />
        {list}
        {entries.hasNextPage ? (
          <Button
            type="button"
            variant="ghost"
            className="h-9 self-start rounded-full px-4"
            disabled={entries.isFetchingNextPage}
            aria-busy={entries.isFetchingNextPage}
            onClick={loadMore}
          >
            {entries.isFetchingNextPage ? <Spinner /> : null}
            {m.configuration_ruleset_members_more()}
          </Button>
        ) : null}
      </div>
    </div>
  )
}

/** Whether the content holds a typed name or address, and the member that decides it. */
function MatchProbe({
  networkId,
  name,
  digest,
  value,
  onChange,
}: {
  networkId: string
  name: string
  digest: string
  value: string
  onChange: (value: string) => void
}) {
  const typed = useSettled(value.trim())
  const match = useQuery({
    ...ruleSetMatchQuery(
      networkId,
      name,
      digest,
      isIp(typed) ? { ip: typed } : { name: typed }
    ),
    enabled: typed !== "",
  })
  let line: ReactNode
  if (typed === "" || value.trim() === "")
    line = (
      <span className="text-muted-foreground">
        {m.configuration_ruleset_match_hint()}
      </span>
    )
  else if (match.data && typed === value.trim()) {
    const member = match.data.member
    line = member ? (
      <>
        <CircleCheckIcon className="size-3.5 shrink-0 text-success" />
        <span className="min-w-0 truncate">
          {m.configuration_ruleset_match_yes({
            query: match.data.query,
            member: memberParts(member).text,
            kind: memberParts(member).kind,
          })}
        </span>
      </>
    ) : (
      <>
        <CircleMinusIcon className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate">
          {m.configuration_ruleset_match_no({ query: match.data.query })}
        </span>
      </>
    )
  } else if (match.isError && typed === value.trim())
    line = (
      <span className="truncate text-destructive">
        {failureText(match.error) ?? m.configuration_ruleset_match_failed()}
      </span>
    )
  else
    line = (
      <span className="text-muted-foreground">
        {m.configuration_ruleset_match_checking()}
      </span>
    )
  return (
    <div className="flex flex-col gap-1.5 rounded-tile bg-fill-subtle p-3">
      <FieldInput
        name="rule-set-match"
        aria-label={m.configuration_ruleset_match()}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        maxLength={254}
        placeholder={m.configuration_ruleset_match_placeholder()}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="h-9 font-mono md:text-meta"
      />
      <p className="flex h-5 items-center gap-1.5 px-1 text-meta">{line}</p>
    </div>
  )
}

/**
 * A provider's sets in the newest catalog the controller verified, by name
 * prefix: each with what a rule writes for it, its size and upstream file,
 * and whether the routing names it. A catalog not verified yet says so.
 */
export function CatalogDialog({
  open,
  onOpenChange,
  networkId,
  provider,
  referenced,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  networkId: string
  provider: string
  referenced: ReadonlySet<string>
}) {
  return (
    <GlassDialog
      open={open}
      onOpenChange={onOpenChange}
      wide
      title={m.configuration_ruleset_browse_title({ provider })}
    >
      {open ? (
        <Catalog
          networkId={networkId}
          provider={provider}
          referenced={referenced}
        />
      ) : null}
    </GlassDialog>
  )
}

function Catalog({
  networkId,
  provider,
  referenced,
}: {
  networkId: string
  provider: string
  referenced: ReadonlySet<string>
}) {
  const [search, setSearch] = useState("")
  const q = useSettled(search.trim().toLowerCase())
  const catalog = useInfiniteQuery(ruleSetCatalogQuery(networkId, provider, q))
  const loadMore = useLoadMore(catalog)
  const pages = catalog.data?.pages
  const head = pages?.[0]
  const items = pages?.flatMap((page) => page.items)

  let list: ReactNode
  if (items === undefined)
    list = catalog.isError ? (
      <p className="rounded-tile bg-fill-subtle px-3.5 py-3 text-row text-muted-foreground">
        {failureText(catalog.error) ?? m.configuration_ruleset_browse_failed()}
      </p>
    ) : (
      <ul className="flex flex-col">
        <ListPlaceholders rows={6} />
      </ul>
    )
  else if (items.length === 0)
    list = <EmptyLine>{m.configuration_ruleset_browse_none()}</EmptyLine>
  else
    list = (
      <ol className="flex max-h-[50vh] flex-col overflow-y-auto overscroll-contain rounded-tile border border-divider">
        {items.map((item) => (
          <li
            key={item.reference}
            className="flex shrink-0 flex-col gap-0.5 border-b border-divider px-3.5 py-2 last:border-b-0"
          >
            <span className="flex min-w-0 items-center gap-2">
              <code className="min-w-0 truncate font-mono text-row">
                {item.reference}
              </code>
              {referenced.has(item.reference) ? (
                <span className="inline-flex h-5 shrink-0 items-center rounded-full bg-fill-selected px-2 text-[11px] font-medium">
                  {m.configuration_ruleset_browse_referenced()}
                </span>
              ) : null}
              <span className="flex-1" />
              <span className="shrink-0 text-meta text-muted-foreground tabular-nums">
                {m.configuration_ruleset_catalog_entries({
                  count: countText(item.entries),
                })}
              </span>
            </span>
            <span className="truncate font-mono text-[11px] text-muted-foreground">
              {[item.upstream.repo, item.upstream.path]
                .filter(Boolean)
                .join(" · ")}
            </span>
            {item.held ? (
              <span className="text-meta text-warning">
                {m.configuration_ruleset_held({
                  reason: item.held.reason,
                  since: dateTime(item.held.since) ?? item.held.since,
                })}
              </span>
            ) : null}
          </li>
        ))}
      </ol>
    )

  return (
    <div className="flex flex-col gap-2">
      <p className="h-5 truncate text-meta text-muted-foreground tabular-nums">
        {head
          ? m.configuration_ruleset_browse_facts({
              total: countText(head.total),
              sequence: head.provider.sequence,
              generated:
                dateTime(head.provider.generatedAt) ??
                head.provider.generatedAt,
              verified:
                dateTime(head.provider.verifiedAt) ?? head.provider.verifiedAt,
            })
          : null}
      </p>
      <FieldInput
        type="search"
        name="rule-set-catalog-search"
        aria-label={m.configuration_ruleset_browse_search()}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        maxLength={128}
        placeholder={m.configuration_ruleset_browse_placeholder()}
        value={search}
        onChange={(event) => setSearch(event.target.value)}
        className={cn("h-9 font-mono md:text-meta")}
      />
      {list}
      {catalog.hasNextPage ? (
        <Button
          type="button"
          variant="ghost"
          className="h-9 self-start rounded-full px-4"
          disabled={catalog.isFetchingNextPage}
          aria-busy={catalog.isFetchingNextPage}
          onClick={loadMore}
        >
          {catalog.isFetchingNextPage ? <Spinner /> : null}
          {m.configuration_ruleset_browse_more()}
        </Button>
      ) : null}
    </div>
  )
}
