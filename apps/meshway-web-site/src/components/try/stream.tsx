"use client"

import {
  listNetworks,
  watchNetwork,
  type ClientFailure,
  type ErrorResponse,
  type NetworkSummary,
} from "@meshway/control-client"
import { Button } from "@meshway/ui/button"
import { cn } from "@meshway/ui/cn"
import { useOpenAPI } from "fumadocs-openapi"
import { ChevronRightIcon } from "lucide-react"
import { useEffect, useRef, useState } from "react"

import type { Lang } from "@/lib/i18n"
import { apiClient, dropTrySession, useTrySession } from "@/lib/try-it"

import { JsonTree } from "./json-tree"
import { Pill } from "./result"

/** Events kept, newest first; older ones are let go. */
const KEPT = 200

const words = {
  zh: {
    network: "网络",
    choose: "选择网络",
    cursor: "从 cursor 续传（可选，Last-Event-ID）",
    connect: "连接",
    stop: "停止",
    clear: "清空",
    annotate: "字段说明",
    idle: "未连接",
    connecting: "正在连接…",
    open: "已连接，等待事件",
    closed: "已断开",
    signIn: "先在上方授权本站，才能连接事件流。",
    events: (count: number) =>
      `${count} 个事件（最多保留 ${KEPT} 个，最新在上）`,
    none: "还没有事件。",
    failed: (why: string) => `连接失败：${why}`,
    decode: "收到的事件不符合契约",
    network_error: "连接不到服务端",
  },
  en: {
    network: "Network",
    choose: "Choose a network",
    cursor: "Resume from a cursor (optional, Last-Event-ID)",
    connect: "Connect",
    stop: "Stop",
    clear: "Clear",
    annotate: "Field notes",
    idle: "Not connected",
    connecting: "Connecting…",
    open: "Connected, waiting for events",
    closed: "Disconnected",
    signIn: "Authorize this site above to connect to the event stream.",
    events: (count: number) =>
      `${count} events (the latest ${KEPT} kept, newest first)`,
    none: "No events yet.",
    failed: (why: string) => `Connection failed: ${why}`,
    decode: "an event did not match the contract",
    network_error: "the server could not be reached",
  },
}

type Event = { seq: number; at: number; id?: string; data: unknown }

type Phase =
  | { kind: "idle" | "connecting" | "open" | "closed" }
  | { kind: "failed"; why: string }

function why(error: ClientFailure<ErrorResponse>, lang: Lang): string {
  const t = words[lang]
  if (error.kind === "http")
    return error.problem
      ? `${error.status} ${error.problem.problem.code}`
      : String(error.status)
  if (error.kind === "decode") return t.decode
  if (error.kind === "network") return t.network_error
  return error.kind
}

const time = (at: number) => {
  const date = new Date(at)
  const pad = (value: number, size = 2) => String(value).padStart(size, "0")
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`
}

const typeOf = (data: unknown) =>
  typeof data === "object" &&
  data !== null &&
  "type" in data &&
  typeof data.type === "string"
    ? data.type
    : "—"

/**
 * Trying an event stream (`watchNetwork`): Fumadocs' playground waits for
 * a whole body, which a stream never ends. Connected with the generated
 * SDK and the delegated session, so each event is checked against the
 * contract; each opens as a tree with its fields' meanings.
 */
export function StreamTry({
  lang,
  eventSchema,
}: {
  lang: Lang
  eventSchema: unknown
}) {
  const t = words[lang]
  const session = useTrySession()
  const { doc } = useOpenAPI()
  const [networks, setNetworks] = useState<NetworkSummary[] | null>(null)
  const [networkId, setNetworkId] = useState("")
  const [cursor, setCursor] = useState("")
  const [phase, setPhase] = useState<Phase>({ kind: "idle" })
  const [events, setEvents] = useState<Event[]>([])
  const [annotate, setAnnotate] = useState(true)
  const [open, setOpen] = useState<number | null>(null)
  const connection = useRef<AbortController | null>(null)
  const seq = useRef(0)

  const token = session?.token ?? null
  useEffect(() => {
    if (!token) return
    let live = true
    void listNetworks(apiClient(), { limit: 200 }, { token }).then((result) => {
      if (!live || !result.ok || !result.data) return
      setNetworks(result.data.items)
      setNetworkId((current) => current || (result.data?.items[0]?.id ?? ""))
    })
    return () => {
      live = false
    }
  }, [token])

  useEffect(() => () => connection.current?.abort(), [])

  const stop = () => {
    connection.current?.abort()
    connection.current = null
    setPhase({ kind: "closed" })
  }

  const connect = async () => {
    if (!session || !networkId) return
    connection.current?.abort()
    const controller = new AbortController()
    connection.current = controller
    setPhase({ kind: "connecting" })
    const resume = cursor.trim()
    for await (const result of watchNetwork(
      apiClient(),
      { networkId },
      {
        token: session.token,
        signal: controller.signal,
        maxReconnects: 0,
        ...(resume ? { resumeCursor: () => resume } : {}),
        onOpen: () => setPhase({ kind: "open" }),
      }
    )) {
      if (controller.signal.aborted) return
      if (!result.ok) {
        if (result.error.kind === "cancelled") return
        if (result.error.kind === "http" && result.error.status === 401)
          dropTrySession()
        setPhase({ kind: "failed", why: why(result.error, lang) })
        return
      }
      seq.current += 1
      const event: Event = {
        seq: seq.current,
        at: Date.now(),
        data: result.data.data,
        ...(result.data.id === undefined ? {} : { id: result.data.id }),
      }
      setEvents((list) => [event, ...list].slice(0, KEPT))
    }
    if (!controller.signal.aborted) setPhase({ kind: "closed" })
  }

  const connected = phase.kind === "connecting" || phase.kind === "open"
  const phaseText =
    phase.kind === "failed" ? t.failed(phase.why) : t[phase.kind]
  const resolve = (node: unknown) => doc.resolve(node)

  return (
    <div className="flex flex-col gap-3 rounded-xl border bg-fd-card p-3.5 text-sm">
      {session ? null : <p className="text-fd-muted-foreground">{t.signIn}</p>}
      <div className="flex flex-wrap items-end gap-2">
        <label className="flex min-w-0 flex-[1_1_14rem] flex-col gap-1">
          <span className="text-xs text-fd-muted-foreground">{t.network}</span>
          <select
            className="h-8 min-w-0 rounded-md border bg-fd-background px-2 text-sm"
            value={networkId}
            disabled={!session || connected}
            onChange={(event) => setNetworkId(event.target.value)}
          >
            {networks === null || networks.length === 0 ? (
              <option value="">{t.choose}</option>
            ) : null}
            {(networks ?? []).map((network) => (
              <option key={network.id} value={network.id}>
                {network.name} · {network.id.slice(0, 8)}
              </option>
            ))}
          </select>
        </label>
        <label className="flex min-w-0 flex-[1_1_14rem] flex-col gap-1">
          <span className="text-xs text-fd-muted-foreground">{t.cursor}</span>
          <input
            className="h-8 min-w-0 rounded-md border bg-fd-background px-2 font-mono text-xs"
            value={cursor}
            disabled={!session || connected}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setCursor(event.target.value)}
          />
        </label>
        {connected ? (
          <Button type="button" variant="outline" size="sm" onClick={stop}>
            {t.stop}
          </Button>
        ) : (
          <Button
            type="button"
            size="sm"
            disabled={!session || !networkId}
            onClick={() => void connect()}
          >
            {t.connect}
          </Button>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={cn(
            "size-2 shrink-0 rounded-full",
            phase.kind === "open"
              ? "bg-green-500"
              : phase.kind === "failed"
                ? "bg-red-500"
                : phase.kind === "connecting"
                  ? "bg-amber-500"
                  : "bg-fd-muted-foreground/40"
          )}
        />
        <span className="text-xs text-fd-muted-foreground" role="status">
          {phaseText} · {t.events(events.length)}
        </span>
        <span className="ms-auto flex items-center gap-1">
          <Pill pressed={annotate} onClick={() => setAnnotate(!annotate)}>
            {t.annotate}
          </Pill>
          <Pill onClick={() => setEvents([])}>{t.clear}</Pill>
        </span>
      </div>
      <div className="max-h-[32rem] overflow-y-auto rounded-lg border bg-fd-background">
        {events.length === 0 ? (
          <p className="p-4 text-xs text-fd-muted-foreground">{t.none}</p>
        ) : (
          <ul className="divide-y">
            {events.map((event) => (
              <li key={event.seq}>
                <button
                  type="button"
                  aria-expanded={open === event.seq}
                  onClick={() => setOpen(open === event.seq ? null : event.seq)}
                  className="flex w-full cursor-pointer items-center gap-3 px-3 py-1.5 text-start font-mono text-xs hover:bg-fd-accent"
                >
                  <ChevronRightIcon
                    className={cn(
                      "size-3 shrink-0 text-fd-muted-foreground transition-transform",
                      open === event.seq && "rotate-90"
                    )}
                  />
                  <span className="shrink-0 text-fd-muted-foreground tabular-nums">
                    {time(event.at)}
                  </span>
                  <span className="shrink-0 font-medium">
                    {typeOf(event.data)}
                  </span>
                  {event.id ? (
                    <span
                      className="min-w-0 truncate text-fd-muted-foreground"
                      title={event.id}
                    >
                      {event.id}
                    </span>
                  ) : null}
                </button>
                {open === event.seq ? (
                  <div className="border-t bg-fd-card px-3 py-2">
                    <JsonTree
                      value={event.data}
                      schema={eventSchema}
                      options={{ lang, resolve, annotate, openDepth: 2 }}
                    />
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
