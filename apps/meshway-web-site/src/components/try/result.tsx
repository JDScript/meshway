"use client"

import { cn } from "@meshway/ui/cn"
import { useOpenAPI } from "fumadocs-openapi"
import { useOperation } from "fumadocs-openapi/operation"
import {
  DefaultResultDisplay,
  type ResultDisplayProps,
} from "fumadocs-openapi/ui/playground/client"
import { XIcon } from "lucide-react"
import { useEffect, useMemo, useState, type ReactNode } from "react"

import type { Lang } from "@/lib/i18n"
import { dropTrySession } from "@/lib/try-it"

import { JsonTree } from "./json-tree"

const words = {
  zh: {
    tree: "解读",
    raw: "原始",
    annotate: "字段说明",
    expand: "全部展开",
    collapse: "全部收起",
    clear: "清除",
    undeclared: "契约没有声明这个状态码，字段没有说明。",
  },
  en: {
    tree: "Annotated",
    raw: "Raw",
    annotate: "Field notes",
    expand: "Expand all",
    collapse: "Collapse all",
    clear: "Clear",
    undeclared:
      "The contract does not declare this status; fields have no notes.",
  },
}

/** The contract's schema of a JSON response with `status`; null when it declares none. */
function useResponseSchema(status: number): unknown {
  const { responses } = useOperation()
  const declared = responses.find((item) => item.status === String(status))
  return declared?.content["application/json"]?.schema ?? null
}

export function Pill({
  pressed,
  onClick,
  children,
}: {
  pressed?: boolean
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      aria-pressed={pressed}
      onClick={onClick}
      className={cn(
        "h-7 cursor-pointer rounded-md px-2 text-xs font-medium text-fd-muted-foreground transition-colors hover:bg-fd-accent hover:text-fd-accent-foreground",
        pressed && "bg-fd-accent text-fd-accent-foreground"
      )}
    >
      {children}
    </button>
  )
}

const statusTone = (status: number) =>
  status < 300 ? "bg-green-500" : status < 500 ? "bg-amber-500" : "bg-red-500"

/**
 * A response as a tree with each field's meaning from the contract, or as
 * Fumadocs shows it raw. A reply saying the session is not taken (401)
 * drops it, so the page asks again.
 */
export function ResultView({
  lang,
  ownErrors,
  ...props
}: ResultDisplayProps & {
  lang: Lang
  /** Messages this site threw before sending (cancelled, not signed in): said plainly. */
  ownErrors: readonly string[]
}) {
  const { data, reset } = props
  const t = words[lang]
  const status = data.type === "response" ? data.status : 0
  const rejected = status === 401
  useEffect(() => {
    if (rejected) dropTrySession()
  }, [rejected])
  const schema = useResponseSchema(status)
  const { doc } = useOpenAPI()
  const json = useMemo(() => {
    if (data.type !== "response") return undefined
    if (!data.mediaType?.includes("json")) return undefined
    try {
      const parsed: unknown = JSON.parse(data.text())
      return { value: parsed }
    } catch {
      return undefined
    }
  }, [data])
  const [view, setView] = useState<"tree" | "raw">("tree")
  const [annotate, setAnnotate] = useState(true)
  const [openDepth, setOpenDepth] = useState(2)
  const [generation, setGeneration] = useState(0)

  if (data.type === "client_error" && ownErrors.includes(data.message))
    return (
      <div
        data-meshway-result
        className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center text-sm text-fd-muted-foreground"
      >
        <p>{data.message}</p>
        <Pill onClick={reset}>{t.clear}</Pill>
      </div>
    )
  if (!json || view === "raw")
    return (
      <div data-meshway-result className="flex min-h-0 flex-1 flex-col">
        {json ? (
          <div className="flex h-9 shrink-0 items-center gap-1 border-b px-2">
            <Pill pressed={false} onClick={() => setView("tree")}>
              {t.tree}
            </Pill>
            <Pill pressed onClick={() => setView("raw")}>
              {t.raw}
            </Pill>
          </div>
        ) : null}
        <DefaultResultDisplay {...props} />
      </div>
    )

  const resolve = (node: unknown) => doc.resolve(node)
  return (
    <div data-meshway-result className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 flex-wrap items-center gap-1 border-b ps-4 pe-1.5">
        <span
          className={cn("size-2 shrink-0 rounded-full", statusTone(status))}
        />
        <span className="me-2 text-[0.8125rem] font-medium tabular-nums">
          {status}
          {data.type === "response" ? (
            <span className="ms-2 font-normal text-fd-muted-foreground">
              {Math.round(data.duration)} ms · {data.body.byteLength} B
            </span>
          ) : null}
        </span>
        <Pill pressed onClick={() => setView("tree")}>
          {t.tree}
        </Pill>
        <Pill pressed={false} onClick={() => setView("raw")}>
          {t.raw}
        </Pill>
        <span className="mx-1 h-4 w-px bg-fd-border" />
        <Pill pressed={annotate} onClick={() => setAnnotate(!annotate)}>
          {t.annotate}
        </Pill>
        <Pill
          onClick={() => {
            setOpenDepth(Number.POSITIVE_INFINITY)
            setGeneration(generation + 1)
          }}
        >
          {t.expand}
        </Pill>
        <Pill
          onClick={() => {
            setOpenDepth(1)
            setGeneration(generation + 1)
          }}
        >
          {t.collapse}
        </Pill>
        <button
          type="button"
          aria-label={t.clear}
          onClick={reset}
          className="ms-auto inline-flex size-7 cursor-pointer items-center justify-center rounded-md text-fd-muted-foreground hover:bg-fd-accent"
        >
          <XIcon className="size-3.5" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {schema === null ? (
          <p className="mb-2 text-xs text-fd-muted-foreground">
            {t.undeclared}
          </p>
        ) : null}
        <JsonTree
          key={generation}
          value={json.value}
          schema={schema}
          options={{ lang, resolve, annotate, openDepth }}
        />
      </div>
    </div>
  )
}
