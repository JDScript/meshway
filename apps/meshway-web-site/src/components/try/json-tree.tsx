"use client"

import { cn } from "@meshway/ui/cn"
import { ChevronRightIcon } from "lucide-react"
import { useState, type ReactNode } from "react"

import type { Lang } from "@/lib/i18n"
import { childSchema, describe, type Resolve } from "@/lib/schema-walk"

// A JSON value as a tree: objects and arrays fold, values are coloured by
// type, and each field says what it means where the contract does.

const words = {
  zh: {
    fields: (count: number) => `${count} 个字段`,
    items: (count: number) => `${count} 项`,
    more: (count: number) => `再显示 ${count} 项`,
  },
  en: {
    fields: (count: number) => (count === 1 ? "1 field" : `${count} fields`),
    items: (count: number) => (count === 1 ? "1 item" : `${count} items`),
    more: (count: number) => `Show ${count} more`,
  },
}

/** Items shown before "show more", at a time. */
const PAGE = 50

export type TreeOptions = {
  lang: Lang
  resolve: Resolve
  /** Show each field's description after its value. */
  annotate: boolean
  /** Levels open at first. */
  openDepth: number
}

function Primitive({ value }: { value: unknown }) {
  if (typeof value === "string")
    return (
      <span className="break-all text-emerald-700 dark:text-emerald-400">
        {JSON.stringify(value)}
      </span>
    )
  if (typeof value === "number")
    return <span className="text-sky-700 dark:text-sky-400">{value}</span>
  return (
    <span className="text-violet-700 dark:text-violet-400">
      {JSON.stringify(value)}
    </span>
  )
}

function Note({ text }: { text: string | null }) {
  if (!text) return null
  return (
    <span
      className="ms-3 min-w-0 flex-1 truncate font-sans text-xs text-fd-muted-foreground/80"
      title={text}
    >
      {text}
    </span>
  )
}

function Row({ depth, children }: { depth: number; children: ReactNode }) {
  return (
    <div
      className="flex min-w-0 items-baseline py-px"
      style={{ paddingInlineStart: `${depth * 14 + 18}px` }}
    >
      {children}
    </div>
  )
}

function Node({
  name,
  value,
  schema,
  depth,
  last,
  options,
}: {
  /** The field name in its object; none for an array item or the root. */
  name: string | null
  value: unknown
  schema: unknown
  depth: number
  last: boolean
  options: TreeOptions
}) {
  const container =
    typeof value === "object" && value !== null
      ? Array.isArray(value)
        ? ("array" as const)
        : ("object" as const)
      : null
  const [open, setOpen] = useState(depth < options.openDepth)
  const [shown, setShown] = useState(PAGE)
  const note =
    options.annotate && name !== null
      ? describe(schema, value, options.resolve)
      : null
  const key =
    name !== null ? (
      <span className="text-fd-foreground">{JSON.stringify(name)}: </span>
    ) : null
  const comma = last ? null : (
    <span className="text-fd-muted-foreground">,</span>
  )

  if (!container)
    return (
      <Row depth={depth}>
        <span className="shrink-0">
          {key}
          <Primitive value={value} />
          {comma}
        </span>
        <Note text={note} />
      </Row>
    )

  const entries: [string | number, unknown][] = Array.isArray(value)
    ? value.map((item: unknown, index) => [index, item])
    : typeof value === "object" && value !== null
      ? Object.entries(value)
      : []
  const [opening, closing] = container === "array" ? ["[", "]"] : ["{", "}"]
  const t = words[options.lang]
  const count =
    container === "array" ? t.items(entries.length) : t.fields(entries.length)

  return (
    <>
      <div
        className="flex min-w-0 items-baseline py-px"
        style={{ paddingInlineStart: `${depth * 14}px` }}
      >
        <button
          type="button"
          aria-expanded={open}
          disabled={entries.length === 0}
          onClick={() => setOpen(!open)}
          className="me-0.5 inline-flex size-4 shrink-0 cursor-pointer items-center justify-center self-center rounded text-fd-muted-foreground hover:bg-fd-accent disabled:invisible"
        >
          <ChevronRightIcon
            className={cn("size-3 transition-transform", open && "rotate-90")}
          />
        </button>
        <span className="shrink-0">
          {key}
          <span className="text-fd-muted-foreground">{opening}</span>
          {open && entries.length > 0 ? null : (
            <>
              {entries.length > 0 ? (
                <button
                  type="button"
                  onClick={() => setOpen(true)}
                  className="mx-1 cursor-pointer rounded bg-fd-accent px-1 font-sans text-xs text-fd-muted-foreground"
                >
                  {count}
                </button>
              ) : null}
              <span className="text-fd-muted-foreground">{closing}</span>
              {comma}
            </>
          )}
        </span>
        <Note text={note} />
      </div>
      {open && entries.length > 0 ? (
        <>
          {entries.slice(0, shown).map(([child, item], index) => (
            <Node
              key={child}
              name={typeof child === "string" ? child : null}
              value={item}
              schema={childSchema(schema, value, child, options.resolve)}
              depth={depth + 1}
              last={index === entries.length - 1}
              options={options}
            />
          ))}
          {entries.length > shown ? (
            <Row depth={depth + 1}>
              <button
                type="button"
                onClick={() => setShown(shown + PAGE)}
                className="cursor-pointer font-sans text-xs text-fd-primary underline-offset-2 hover:underline"
              >
                {t.more(Math.min(PAGE, entries.length - shown))}
              </button>
            </Row>
          ) : null}
          <div
            className="py-px"
            style={{ paddingInlineStart: `${depth * 14 + 18}px` }}
          >
            <span className="text-fd-muted-foreground">{closing}</span>
            {comma}
          </div>
        </>
      ) : null}
    </>
  )
}

/** `value` as a tree; `schema` is what the contract says it is (null when it says nothing). */
export function JsonTree({
  value,
  schema,
  options,
}: {
  value: unknown
  schema: unknown
  options: TreeOptions
}) {
  return (
    <div className="font-mono text-[0.8125rem] leading-relaxed">
      <Node
        name={null}
        value={value}
        schema={schema}
        depth={0}
        last
        options={options}
      />
    </div>
  )
}
