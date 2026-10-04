import {
  createEnrollmentKey,
  revokeEnrollmentKey,
  type CreateEnrollmentKeyRequest,
  type EnrollmentKeyResponse,
  type EnrollmentKeyView,
  type ListenerView,
  type NetworkSummary,
  type NodeInventoryResponse,
} from "@meshway/control-client"
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query"
import { Link } from "@tanstack/react-router"
import {
  ArrowUpRightIcon,
  CheckIcon,
  ChevronDownIcon,
  KeyRoundIcon,
  XIcon,
} from "lucide-react"
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { toast } from "sonner"

import { authed, client } from "@/api/client"
import { failureText, toastFailureWith } from "@/api/failure"
import { configurationQuery, isDenied } from "@/api/queries/configuration"
import { membersQuery } from "@/api/queries/members"
import { enrollmentKeysQuery, transportCatalogQuery } from "@/api/queries/nodes"
import { BusyLabel } from "@/components/chrome/busy-label"
import { CloseButton } from "@/components/chrome/close-button"
import { FIELD_SIZE, TextField } from "@/components/chrome/form"
import {
  ConfirmDialog,
  GlassDialog,
  SecretField,
} from "@/components/chrome/glass-dialog"
import { EmptyLine, Section } from "@/components/chrome/section"
import { SegmentedControl } from "@/components/chrome/segmented-control"
import { SurfaceHeader } from "@/components/chrome/surface-header"
import { Tile } from "@/components/chrome/tile"
import { surfaceTitleId } from "@/components/surface/page-frame"
import { Button } from "@/components/ui/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { Skeleton } from "@/components/ui/skeleton"
import { useLoadMore } from "@/hooks/use-load-more"
import { useNow } from "@/hooks/use-now"
import { useSecretMutation } from "@/hooks/use-secret-mutation"
import { dateTime } from "@/lib/format"
import { presetOfView } from "@/lib/nodes/listener-edit"
import {
  advancedSet,
  endpointText,
  LISTENER_KEY,
  listenerOf,
  listenerProblems,
  newListener,
  presetsIn,
  problemField,
  protocolOf,
  suggestedPort,
  withPreset,
  type ListenerField,
  type ListenerFields,
  type ListenerProblem,
  type Preset,
} from "@/lib/nodes/listener-model"
import {
  enrollmentKeyState,
  hostnameOf,
  KEY_EXPIRIES,
  type KeyExpiry,
} from "@/lib/nodes/model"
import { cn } from "@/lib/utils"
import { m } from "@/paraglide/messages.js"
import { attempts } from "@/state/attempts"

import {
  expiryLabel,
  keyStateLabel,
  presetLabel,
  protocolLabel,
} from "../-labels"

/** Long enough for the dialog to finish closing before the key is dropped. */
const UNMOUNT_AFTER_MS = 400
const NONE = "none"
type Choice = Preset | typeof NONE

type Body = Omit<CreateEnrollmentKeyRequest, "requestId">

/** By network. */
const enrollmentKeys = attempts<Body>()

const fieldLabelClass = cn("leading-snug font-medium", FIELD_SIZE.default.label)

export const listenerProblemText: Record<ListenerProblem, () => string> = {
  port: () => m.nodes_listener_port_invalid(),
  host: () => m.nodes_listener_host_invalid(),
  public_port: () => m.nodes_listener_public_port_invalid(),
  listen: () => m.nodes_listener_listen_invalid(),
  server_name: () => m.nodes_listener_server_name_invalid(),
  server_name_missing: () => m.nodes_listener_server_name_missing(),
}

/**
 * Adding a node: an enrolment key for one Tag-identity device, with an
 * optional hostname and one optional listener, then the network's keys.
 * The key is shown once, in a dialog that drops it when it closes; this
 * controller provides no Agent install, and the page says so.
 */
export function AddNode({
  networkId,
  network,
  nodes,
  onClose,
  onOpenNode,
}: {
  networkId: string
  network: NetworkSummary | null
  nodes: NodeInventoryResponse | undefined
  onClose: () => void
  onOpenNode: (nodeId: string) => void
}) {
  const queryClient = useQueryClient()
  const configuration = useQuery(configurationQuery(networkId))
  const catalog = useQuery(transportCatalogQuery())
  const tagNames = useMemo(
    () =>
      configuration.data
        ? Object.keys(configuration.data.configuration.tagsOwners).sort()
        : null,
    [configuration.data]
  )
  const presets = useMemo(
    () =>
      catalog.data
        ? presetsIn(catalog.data.items.map((item) => item.protocol))
        : [],
    [catalog.data]
  )

  const [tags, setTags] = useState<readonly string[]>([])
  const [hostname, setHostname] = useState("")
  const [expiry, setExpiry] = useState<KeyExpiry>(86_400)
  const [choice, setChoice] = useState<Choice>(NONE)
  // Kept while "no listener" is chosen, so nothing typed is lost.
  const [fields, setFields] = useState<ListenerFields>(() =>
    newListener("native_udp")
  )
  const [attempted, setAttempted] = useState(false)

  // Tags that left the configuration are no longer chosen.
  const chosen = tagNames ? tags.filter((tag) => tagNames.includes(tag)) : tags
  const name = hostnameOf(hostname)
  const listenerIssues = choice === NONE ? [] : listenerProblems(fields)
  const problem =
    chosen.length === 0
      ? m.nodes_add_tags_missing()
      : name === null
        ? m.nodes_add_hostname_invalid()
        : listenerIssues[0]
          ? listenerProblemText[listenerIssues[0]]()
          : null
  const invalid = new Set<ListenerField>(
    attempted ? listenerIssues.map((issue) => problemField[issue]) : []
  )

  const secret = useSecretMutation(
    ({ key, body }: { key: string; body: () => Body }) =>
      enrollmentKeys.send(
        networkId,
        key,
        body,
        (attempt): Promise<EnrollmentKeyResponse> =>
          authed((token) =>
            createEnrollmentKey(
              client,
              { networkId },
              { requestId: attempt.requestId, ...attempt.body },
              { token }
            )
          )
      ),
    {
      permission_denied: () => m.nodes_manage_denied(),
      invalid_request: () => m.nodes_add_refused(),
    }
  )

  // Shown from the moment the key arrives until the dialog is closed; the
  // key is dropped once the dialog has finished closing.
  const created = secret.result
  const [dismissed, setDismissed] = useState(false)
  const { clear } = secret
  const closing = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(closing.current), [])
  const dismiss = () => {
    setDismissed(true)
    window.clearTimeout(closing.current)
    closing.current = window.setTimeout(() => {
      clear()
      setDismissed(false)
    }, UNMOUNT_AFTER_MS)
  }
  useEffect(() => {
    if (created)
      void queryClient.invalidateQueries({
        queryKey: enrollmentKeysQuery(networkId).queryKey,
      })
  }, [created, queryClient, networkId])

  const submit = () => {
    setAttempted(true)
    if (problem !== null || name === null || secret.pending) return
    const listener = choice === NONE ? null : listenerOf(fields)
    if (choice !== NONE && listener === null) return
    const input = {
      tags: [...chosen].sort(),
      hostname: name,
      expiry,
      listener:
        choice === NONE ? null : { key: LISTENER_KEY[choice], listener },
    }
    secret.create({
      key: JSON.stringify(input),
      body: () => ({
        tags: input.tags,
        initialSettings: {
          ...(name === "" ? {} : { hostname: name }),
          listeners:
            input.listener && input.listener.listener
              ? { [input.listener.key]: input.listener.listener }
              : {},
        },
        expiresAt: new Date(Date.now() + expiry * 1000).toISOString(),
      }),
    })
  }

  const toggle = (tag: string) =>
    setTags((current) =>
      current.includes(tag)
        ? current.filter((entry) => entry !== tag)
        : [...current, tag]
    )

  return (
    <div className="swap-in flex min-h-0 flex-1 flex-col">
      <SurfaceHeader
        size="lg"
        className="px-6 pt-6 pb-4"
        title={(className) => (
          <h2 id={surfaceTitleId("nodes", "workspace")} className={className}>
            {m.nodes_add()}
          </h2>
        )}
        action={<CloseButton onClick={onClose} />}
      />
      <form
        noValidate
        className="flex min-h-0 flex-1 flex-col"
        onSubmit={(event) => {
          event.preventDefault()
          submit()
        }}
      >
        <div className="scroll-edge min-h-0 flex-1 overflow-y-auto px-6 pt-1 pb-6">
          <div className="flex max-w-workspace flex-col gap-8">
            <Section title={m.nodes_add_tags()} count={chosen.length}>
              {tagNames === null ? (
                configuration.isError ? (
                  <EmptyLine>
                    {isDenied(configuration.error)
                      ? m.nodes_listeners_denied()
                      : (failureText(configuration.error) ??
                        m.nodes_listeners_failed())}
                  </EmptyLine>
                ) : (
                  <Skeleton className="h-12 w-full rounded-tile" />
                )
              ) : tagNames.length === 0 ? (
                <EmptyLine>
                  <span className="min-w-0 flex-1 truncate">
                    {m.nodes_add_tags_none()}
                  </span>
                  <Link
                    to="/networks/$network/configuration/$view"
                    params={{ network: networkId, view: "source" }}
                    search={{ at: "/tagsOwners" }}
                    className="shrink-0 font-medium text-foreground underline-offset-4 hover:underline"
                  >
                    {m.nodes_add_tags_define()}
                  </Link>
                </EmptyLine>
              ) : (
                <fieldset className="flex flex-wrap gap-1.5">
                  <legend className="sr-only">{m.nodes_add_tags()}</legend>
                  {tagNames.map((tag) => {
                    const on = chosen.includes(tag)
                    return (
                      <button
                        key={tag}
                        type="button"
                        aria-pressed={on}
                        onClick={() => toggle(tag)}
                        data-invalid={
                          (attempted && chosen.length === 0) || undefined
                        }
                        className="inline-flex h-8 items-center gap-1.5 rounded-full border border-border px-3 font-mono text-meta transition-colors outline-none hover:bg-fill-hover focus-visible:ring-2 focus-visible:ring-ring aria-pressed:border-transparent aria-pressed:bg-fill-selected aria-pressed:text-foreground aria-pressed:hover:bg-fill-selected data-invalid:border-warning [&:not([aria-pressed=true])]:text-muted-foreground"
                      >
                        {on ? <CheckIcon className="-ml-0.5 size-3.5" /> : null}
                        {tag}
                      </button>
                    )
                  })}
                </fieldset>
              )}
            </Section>

            <div className="grid gap-4 sm:grid-cols-[1fr_auto]">
              <TextField
                id="add-node-hostname"
                name="hostname"
                label={m.nodes_add_hostname()}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                maxLength={63}
                className="font-mono placeholder:font-sans"
                placeholder={m.nodes_add_hostname_placeholder()}
                value={hostname}
                invalid={attempted && name === null}
                onChange={(event) => setHostname(event.target.value)}
              />
              <div className="flex flex-col gap-2">
                <span className={fieldLabelClass}>{m.nodes_add_expiry()}</span>
                <SegmentedControl<`${KeyExpiry}`>
                  label={m.nodes_add_expiry()}
                  value={`${expiry}`}
                  options={KEY_EXPIRIES.map((seconds) => ({
                    value: `${seconds}`,
                    label: expiryLabel[seconds](),
                  }))}
                  onChange={(value) => {
                    const next = KEY_EXPIRIES.find(
                      (seconds) => `${seconds}` === value
                    )
                    if (next !== undefined) setExpiry(next)
                  }}
                  className="h-10 self-start"
                />
              </div>
            </div>

            <Section title={m.nodes_add_listener()}>
              <div className="flex flex-col gap-4">
                {catalog.data === undefined ? (
                  catalog.isError ? (
                    <EmptyLine>
                      {failureText(catalog.error) ?? m.nodes_catalog_failed()}
                    </EmptyLine>
                  ) : (
                    <Skeleton className="h-9 w-80 rounded-full" />
                  )
                ) : (
                  <SegmentedControl<Choice>
                    label={m.nodes_add_listener()}
                    value={choice}
                    options={[
                      { value: NONE, label: m.nodes_add_listener_none() },
                      ...presets.map((preset) => ({
                        value: preset,
                        label: presetLabel[preset](),
                      })),
                    ]}
                    onChange={(next) => {
                      setChoice(next)
                      if (next !== NONE)
                        setFields((current) => withPreset(current, next))
                    }}
                    className="self-start"
                  />
                )}
                {choice !== NONE ? (
                  <ListenerInputs
                    fields={fields}
                    invalid={invalid}
                    onChange={setFields}
                  />
                ) : null}
              </div>
            </Section>

            <Section title={m.nodes_install()}>
              <EmptyLine>{m.nodes_install_none()}</EmptyLine>
            </Section>

            <EnrollmentKeys
              networkId={networkId}
              network={network}
              nodes={nodes}
              onOpenNode={onOpenNode}
            />
          </div>
        </div>
        <footer className="flex h-16 shrink-0 items-center gap-3 border-t border-divider px-6">
          <span className="min-w-0 flex-1 truncate text-row text-warning">
            {attempted ? problem : null}
          </span>
          <Button
            type="submit"
            className="h-9 rounded-full px-4"
            disabled={secret.pending || tagNames === null}
            aria-busy={secret.pending}
          >
            <BusyLabel busy={secret.pending}>{m.nodes_add_submit()}</BusyLabel>
          </Button>
        </footer>
      </form>

      {created ? (
        <KeyDialog open={!dismissed} created={created} onClose={dismiss} />
      ) : null}
    </div>
  )
}

/** A listener's reachability inputs: public address, port, TLS name, and under "advanced" the bind address and a public port of its own. */
export function ListenerInputs({
  fields,
  invalid,
  onChange,
  idPrefix = "add-listener",
  generated = true,
  children,
}: {
  fields: ListenerFields
  invalid: ReadonlySet<ListenerField>
  onChange: (fields: ListenerFields) => void
  /** Prefixes the fields' IDs, unique on the page. */
  idPrefix?: string
  /** Says the rest is generated (a new node's listener). */
  generated?: boolean
  /** More inputs under "advanced". */
  children?: ReactNode
}) {
  const set = (key: ListenerField, value: string) =>
    onChange({ ...fields, [key]: value })
  const vless = protocolOf(fields.preset) === "vless"
  // Opens by itself when something inside is wrong; it can still be closed.
  const hidden = invalid.has("listen") || invalid.has("publicPort")
  const [open, setOpen] = useState(advancedSet(fields))
  const [seen, setSeen] = useState(hidden)
  if (hidden !== seen) {
    setSeen(hidden)
    if (hidden) setOpen(true)
  }
  const host = fields.host.trim()

  return (
    <div className="swap-in flex flex-col gap-4">
      <div className="grid grid-cols-[minmax(0,1fr)_6rem] gap-x-2 gap-y-4">
        <TextField
          id={`${idPrefix}-host`}
          name="public-host"
          label={m.nodes_listener_host()}
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={253}
          className="font-mono placeholder:font-sans"
          placeholder={m.nodes_listener_host_placeholder()}
          value={fields.host}
          invalid={invalid.has("host")}
          onChange={(event) => set("host", event.target.value)}
        />
        <TextField
          id={`${idPrefix}-port`}
          name="port"
          label={m.nodes_listener_port()}
          inputMode="numeric"
          autoComplete="off"
          maxLength={5}
          className="font-mono tabular-nums"
          placeholder={suggestedPort(fields.preset)}
          value={fields.port}
          invalid={invalid.has("port")}
          onChange={(event) => set("port", event.target.value.trim())}
        />
        {vless ? (
          <div className="col-span-2">
            <TextField
              id={`${idPrefix}-server-name`}
              name="server-name"
              label={m.nodes_listener_server_name()}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              maxLength={253}
              className="font-mono"
              placeholder={host || m.nodes_listener_server_name_placeholder()}
              value={fields.serverName}
              invalid={invalid.has("serverName")}
              onChange={(event) => set("serverName", event.target.value)}
            />
          </div>
        ) : null}
      </div>

      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="list-row -mx-2 h-10 w-[calc(100%+1rem)] gap-3 px-2">
          <span className="shrink-0 text-row font-medium">
            {m.nodes_listener_advanced()}
          </span>
          <span className="min-w-0 flex-1 truncate text-left text-meta text-muted-foreground">
            {advancedSet(fields)
              ? [
                  fields.listen.trim() &&
                    m.nodes_listener_fact_listen({
                      address: fields.listen.trim(),
                    }),
                  fields.publicPort.trim() &&
                    m.nodes_listener_fact_public_port({
                      port: fields.publicPort.trim(),
                    }),
                ]
                  .filter(Boolean)
                  .join(" · ")
              : m.nodes_listener_defaults()}
          </span>
          <ChevronDownIcon
            className="size-4 shrink-0 text-muted-foreground transition-transform duration-200 data-[open=true]:rotate-180"
            data-open={open}
          />
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="grid grid-cols-[minmax(0,1fr)_6rem] gap-x-2 pt-3">
            <TextField
              id={`${idPrefix}-listen`}
              name="listen"
              label={m.nodes_listener_listen()}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              maxLength={45}
              className="font-mono"
              placeholder="0.0.0.0"
              value={fields.listen}
              invalid={invalid.has("listen")}
              onChange={(event) => set("listen", event.target.value)}
            />
            <TextField
              id={`${idPrefix}-public-port`}
              name="public-port"
              label={m.nodes_listener_public_port()}
              inputMode="numeric"
              autoComplete="off"
              maxLength={5}
              className="font-mono tabular-nums"
              placeholder={fields.port || suggestedPort(fields.preset)}
              value={fields.publicPort}
              invalid={invalid.has("publicPort")}
              disabled={host === ""}
              onChange={(event) => set("publicPort", event.target.value.trim())}
            />
          </div>
          {children}
        </CollapsibleContent>
      </Collapsible>
      {generated ? (
        <p className="text-meta text-muted-foreground">
          {m.nodes_listener_generated()}
        </p>
      ) : null}
    </div>
  )
}

/** The key just created: shown once, here only. */
function KeyDialog({
  open,
  created,
  onClose,
}: {
  open: boolean
  created: EnrollmentKeyResponse
  onClose: () => void
}) {
  const secret =
    created.secretDelivery === "available" ? (created.secret ?? null) : null
  return (
    <GlassDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      title={m.nodes_key_created()}
      footer={
        <Button className="h-9 rounded-full px-4" onClick={onClose}>
          {m.nodes_done()}
        </Button>
      }
    >
      <div className="grid grid-cols-2 gap-2">
        <Tile label={m.nodes_add_tags()}>
          <span className="font-mono font-normal">
            {created.key.tags.join(", ")}
          </span>
        </Tile>
        <Tile label={m.nodes_key_expires()}>
          {dateTime(created.key.expiresAt) ?? "—"}
        </Tile>
      </div>
      {secret ? (
        <SecretField label={m.nodes_key_secret()} value={secret} />
      ) : (
        <p className="text-row text-muted-foreground">
          {m.nodes_key_unavailable()}
        </p>
      )}
      <p className="text-row text-muted-foreground">{m.nodes_install_none()}</p>
    </GlassDialog>
  )
}

/** A listener of a key's initial settings, as one line: its kind and where it is reached (else bound). */
function listenerLine(name: string, view: ListenerView) {
  const preset = presetOfView(view)
  const kind = preset ? presetLabel[preset]() : protocolLabel[view.protocol]()
  const endpoint = view.endpoints[0]
  return `${name} · ${kind} · ${
    endpoint
      ? endpointText(endpoint.host, endpoint.port)
      : endpointText(view.listen, view.listenPort)
  }`
}

/**
 * The network's enrolment keys, newest first as the controller pages them,
 * with its total: what each enrols (tags, hostname, listeners), who
 * created it, and its state; a used key names the node it became.
 */
function EnrollmentKeys({
  networkId,
  network,
  nodes,
  onOpenNode,
}: {
  networkId: string
  network: NetworkSummary | null
  nodes: NodeInventoryResponse | undefined
  onOpenNode: (nodeId: string) => void
}) {
  const queryClient = useQueryClient()
  const keys = useInfiniteQuery(enrollmentKeysQuery(networkId))
  const loadMore = useLoadMore(keys)
  // The creators' emails, while this account may read the members.
  const members = useInfiniteQuery({
    ...membersQuery(networkId),
    enabled: network?.actions.manageMembers ?? false,
  })
  const emails = useMemo(
    () =>
      new Map(
        (members.data?.pages ?? []).flatMap((page) =>
          page.items.map((member) => [member.userId, member.email] as const)
        )
      ),
    [members.data]
  )
  const now = useNow(30_000)
  const [revoking, setRevoking] = useState<EnrollmentKeyView | null>(null)
  const [confirming, setConfirming] = useState(false)
  const revoke = useMutation({
    mutationFn: (keyId: string) =>
      authed((token) =>
        revokeEnrollmentKey(client, { networkId, keyId }, { token })
      ),
    onSuccess: async (_, keyId) => {
      setConfirming(false)
      // The controller answers alike when the key was used or expired
      // first; what it holds now says whether it was revoked.
      const read = await keys.refetch()
      const key = read.data?.pages
        .flatMap((page) => page.items)
        .find((item) => item.id === keyId)
      if (key && key.revokedAt === null)
        toast.warning(m.nodes_enrollment_revoke_moot())
      else toast.success(m.nodes_enrollment_revoke_done())
    },
    onError: (error) => {
      setConfirming(false)
      toastFailureWith({ permission_denied: () => m.nodes_manage_denied() })(
        error
      )
      void queryClient.invalidateQueries({
        queryKey: enrollmentKeysQuery(networkId).queryKey,
      })
    },
  })

  const pages = keys.data?.pages
  const rows = useMemo(
    () =>
      (pages ?? [])
        .flatMap((page) => page.items)
        .map((key) => ({ key, ...enrollmentKeyState(key, now) })),
    [pages, now]
  )
  const nodeName = (nodeId: string) =>
    nodes?.items.find((node) => node.id === nodeId)?.hostname ?? null

  let body: ReactNode
  if (pages === undefined)
    body = keys.isError ? (
      <EmptyLine>
        {failureText(keys.error) ?? m.nodes_enrollment_failed()}
      </EmptyLine>
    ) : (
      <Skeleton className="h-12 w-full rounded-tile" />
    )
  else if (rows.length === 0)
    body = <EmptyLine>{m.nodes_enrollment_none()}</EmptyLine>
  else
    body = (
      <ul className="flex flex-col gap-0.5">
        {rows.map(({ key, state, at }) => {
          const listeners = Object.entries(key.initialSettings.listeners).sort(
            ([a], [b]) => a.localeCompare(b)
          )
          const node = key.nodeId
          const name = node === null ? null : nodeName(node)
          const when =
            state === "consumed"
              ? at === null
                ? null
                : m.nodes_enrollment_consumed_at({ at: dateTime(at) ?? "—" })
              : state === "revoked"
                ? m.nodes_enrollment_revoked_at({
                    at: dateTime(at ?? "") ?? "—",
                  })
                : m.nodes_enrollment_expires({
                    at: dateTime(key.expiresAt) ?? "—",
                  })
          return (
            <li
              key={key.id}
              className="flex min-h-12 w-full items-start gap-3 rounded-tile py-2 pr-2 pl-3.5"
            >
              <span className="flex h-5 w-4 shrink-0 items-center justify-center">
                <KeyRoundIcon
                  className={cn(
                    "size-4",
                    state !== "pending" && "text-muted-foreground"
                  )}
                />
              </span>
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate text-row font-medium">
                  <span className="font-mono font-normal">
                    {key.tags.join(", ")}
                  </span>
                  {key.initialSettings.hostname !== null ? (
                    <span className="font-mono font-normal text-muted-foreground">
                      {" · "}
                      {key.initialSettings.hostname}
                    </span>
                  ) : null}
                </span>
                <span className="text-meta break-words text-muted-foreground">
                  {[
                    m.nodes_enrollment_created_by({
                      at: dateTime(key.createdAt) ?? "—",
                      user: emails.get(key.createdBy) ?? key.createdBy,
                    }),
                    when,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
                <span className="text-meta break-words text-muted-foreground">
                  {[
                    key.initialSettings.hostname === null
                      ? m.nodes_enrollment_hostname_device()
                      : null,
                    listeners.length === 0
                      ? m.nodes_enrollment_listeners_none()
                      : listeners
                          .map(([listener, view]) =>
                            listenerLine(listener, view)
                          )
                          .join("; "),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
                {node !== null ? (
                  name !== null ? (
                    <button
                      type="button"
                      onClick={() => onOpenNode(node)}
                      className="inline-flex items-center gap-1 self-start rounded-sm text-meta font-medium text-foreground underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      {m.nodes_enrollment_node({ name })}
                      <ArrowUpRightIcon className="size-3.5" />
                    </button>
                  ) : (
                    <span className="text-meta break-all text-muted-foreground">
                      {m.nodes_enrollment_node_gone({ id: node })}
                    </span>
                  )
                ) : null}
              </span>
              <span className="flex h-5 shrink-0 items-center text-meta text-muted-foreground">
                {keyStateLabel[state]()}
              </span>
              <span className="flex size-8 shrink-0 items-center justify-center">
                {key.actions.revoke ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-sm"
                    className="-mt-1.5 rounded-full"
                    aria-label={m.nodes_enrollment_revoke()}
                    onClick={() => {
                      setRevoking(key)
                      setConfirming(true)
                    }}
                  >
                    <XIcon />
                  </Button>
                ) : null}
              </span>
            </li>
          )
        })}
        {keys.hasNextPage ? (
          <li>
            <Button
              type="button"
              variant="ghost"
              className="h-10 w-full justify-start rounded-tile pl-3.5 text-muted-foreground"
              disabled={keys.isFetchingNextPage}
              onClick={loadMore}
            >
              <BusyLabel busy={keys.isFetchingNextPage}>
                {m.nodes_enrollment_more()}
              </BusyLabel>
            </Button>
          </li>
        ) : null}
      </ul>
    )

  return (
    <Section
      title={m.nodes_enrollment_keys()}
      count={pages ? (pages.at(-1)?.total ?? null) : null}
    >
      {body}
      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          if (!revoke.isPending) setConfirming(open)
        }}
        title={m.nodes_enrollment_revoke()}
        confirmLabel={m.nodes_enrollment_revoke()}
        pending={revoke.isPending}
        onConfirm={() => {
          if (revoking) revoke.mutate(revoking.id)
        }}
        effects={[
          m.nodes_enrollment_revoke_effect({
            tags: revoking ? revoking.tags.join(", ") : "",
          }),
          m.nodes_enrollment_revoke_effect_final(),
        ]}
      />
    </Section>
  )
}
