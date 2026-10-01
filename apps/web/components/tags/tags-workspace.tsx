"use client"

import { useRoutes } from "@/components/shell/network-context"
import { useNetworkId } from "@/components/network/network-workspace"

import type { ConnectionNode, MeshAccessSnapshot, TagDefinition, TagSnapshot } from "@meshway/control-client"
import { CircleAlertIcon, PencilIcon, PlusIcon, RefreshCwIcon, RotateCcwIcon, SaveIcon, TagIcon, Trash2Icon } from "lucide-react"
import Link from "next/link"
import { useEffect, useId, useMemo, useState, useTransition } from "react"

import { Panel } from "@/components/common/panel"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Spinner } from "@/components/ui/spinner"
import type { TagSaveResult } from "@/lib/action-state"
import { reloadTagsAction, saveTagsAction, setNodeTagsAction } from "@/lib/tags-actions"
import {
  catalogKey,
  deletionBlocker,
  membership,
  normalizeTagInput,
  TAG_LIMITS,
  tagsOfNode,
  validateCatalog,
} from "@/lib/tags-model"
import { cn } from "@/lib/utils"

type Feedback = { scope: "catalog" | string; result: TagSaveResult } | null

/**
 * Tag catalog and node bindings. Each save is a conditional write against
 * the global configuration revision held here, which only moves with this
 * page's own successful saves or an explicit reload; a conflict leaves the
 * drafts untouched.
 */
export function TagsWorkspace({
  snapshot: initialSnapshot,
  inventory: initialInventory,
  access: initialAccess,
  focusNodeId,
}: {
  snapshot: TagSnapshot
  inventory: ConnectionNode[]
  access: MeshAccessSnapshot | null
  focusNodeId: string | null
}) {
  const routes = useRoutes()
  const [snapshot, setSnapshot] = useState(initialSnapshot)
  const networkId = useNetworkId()
  const [inventory, setInventory] = useState(initialInventory)
  const [access, setAccess] = useState(initialAccess)
  const [catalog, setCatalog] = useState<TagDefinition[]>(initialSnapshot.tags)
  const [editingNode, setEditingNode] = useState<string | null>(focusNodeId)
  const [nodeDraft, setNodeDraft] = useState<string[]>(() => (focusNodeId ? tagsOfNode(initialSnapshot, focusNodeId) : []))
  const [feedback, setFeedback] = useState<Feedback>(null)
  const [reloadError, setReloadError] = useState<string | null>(null)
  const [pending, start] = useTransition()
  const [newName, setNewName] = useState("")
  const [newDescription, setNewDescription] = useState("")
  const nameId = useId()
  const descriptionId = useId()

  const catalogDirty = catalogKey(catalog) !== catalogKey(snapshot.tags)
  const catalogIssues = validateCatalog(catalog)
  const saved = new Set(snapshot.tags.map((definition) => definition.tag))
  const members = useMemo(() => membership(snapshot, inventory), [snapshot, inventory])
  const active = inventory.filter((entry) => entry.node.revoked_at === null)
  const candidate = normalizeTagInput(newName)
  const candidateIssues = newName.trim() === "" ? [] : validateCatalog([...catalog, { tag: candidate, description: newDescription }]).filter((issue) => issue.index === catalog.length)

  const dirty = catalogDirty || editingNode !== null
  useEffect(() => {
    if (!dirty) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [dirty])

  const accept = (result: TagSaveResult, scope: string) => {
    setFeedback({ scope, result })
    if (result.status === "saved") setSnapshot(result.snapshot)
    return result.status === "saved"
  }

  const saveCatalog = () => {
    const expected = snapshot.revision
    const body = catalog
    start(async () => {
      const result = await saveTagsAction(networkId, expected, body)
      if (accept(result, "catalog") && result.status === "saved") setCatalog(result.snapshot.tags)
    })
  }

  const saveNode = (nodeId: string) => {
    const expected = snapshot.revision
    const body = nodeDraft
    start(async () => {
      const result = await setNodeTagsAction(networkId, expected, nodeId, body)
      if (accept(result, nodeId)) setEditingNode(null)
    })
  }

  const reload = () => {
    start(async () => {
      const result = await reloadTagsAction(networkId)
      if (result.status === "error") {
        setReloadError(result.message)
        return
      }
      setReloadError(null)
      setSnapshot(result.snapshot)
      setInventory(result.inventory)
      setAccess(result.access)
      setCatalog(result.snapshot.tags)
      setEditingNode(null)
      setFeedback(null)
    })
  }

  const addTag = () => {
    if (newName.trim() === "" || candidateIssues.length > 0) return
    setCatalog((current) => [...current, { tag: candidate, description: newDescription.trim() }])
    setNewName("")
    setNewDescription("")
  }

  const startNodeEdit = (nodeId: string) => {
    setEditingNode(nodeId)
    setNodeDraft(tagsOfNode(snapshot, nodeId))
    setFeedback(null)
  }

  return (
    <div data-testid="tags-workspace" className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-xl border bg-card px-4 py-2.5 text-xs">
        <span className="flex items-center gap-1.5">
          <span className="text-muted-foreground">编辑基础</span>
          <span data-testid="tags-base-revision" className="font-mono tabular-nums">
            r{snapshot.revision}
          </span>
        </span>
        <span className="flex items-center gap-1.5">
          <span className="text-muted-foreground">标签</span>
          <span className="tabular-nums">
            {snapshot.tags.length}/{TAG_LIMITS.catalog}
          </span>
        </span>
        <span className="text-muted-foreground">保存标签或节点标签会推进全局配置版本；其他页面的未保存草稿需要重新读取后再保存。</span>
        <Button type="button" variant="ghost" size="sm" className="ml-auto" disabled={pending} onClick={reload}>
          <RefreshCwIcon data-icon="inline-start" className={pending ? "animate-spin" : undefined} />
          {dirty ? "放弃修改并重新读取" : "重新读取"}
        </Button>
      </div>
      {reloadError ? (
        <p role="alert" className="flex items-start gap-1.5 text-xs text-destructive">
          <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
          <span className="break-all">{reloadError}</span>
        </p>
      ) : null}
      <FeedbackLine feedback={feedback} onReload={reload} />

      <div className="grid items-start gap-4 xl:grid-cols-2">
        <Panel
          data-testid="tag-catalog"
          title="标签目录"
          description="删除前需先移除节点上的标签和规则中的引用。"
          bodyClassName="flex flex-col"
        >
          {catalog.length === 0 ? (
            <Empty className="py-8">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <TagIcon />
                </EmptyMedia>
                <EmptyTitle>还没有标签</EmptyTitle>
                <EmptyDescription>在下方添加，例如 edge、core。</EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            <ul aria-label="标签目录" className="divide-y">
              {catalog.map((definition, index) => {
                const blocker = saved.has(definition.tag) ? deletionBlocker(definition.tag, snapshot, access?.policy ?? null) : null
                const tagMembers = members.get(definition.tag) ?? []
                const issues = catalogIssues.filter((issue) => issue.index === index)
                return (
                  <li key={definition.tag} data-testid="tag-row" className="flex flex-col gap-1.5 px-4 py-3">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-sm font-medium">{definition.tag}</span>
                      {!saved.has(definition.tag) ? <Badge variant="secondary">新增</Badge> : null}
                      <span className="ml-auto text-xs text-muted-foreground tabular-nums">{tagMembers.length} 个成员</span>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`删除标签 ${definition.tag}`}
                        title={blocker ?? undefined}
                        disabled={pending || blocker !== null}
                        onClick={() => setCatalog((current) => current.filter((_, position) => position !== index))}
                      >
                        <Trash2Icon />
                      </Button>
                    </div>
                    <Input
                      aria-label={`${definition.tag} 的说明`}
                      value={definition.description}
                      placeholder="说明（可选）"
                      maxLength={TAG_LIMITS.description}
                      onChange={(event) => {
                        const description = event.target.value
                        setCatalog((current) =>
                          current.map((item, position) => (position === index ? { ...item, description } : item))
                        )
                      }}
                      disabled={pending}
                      className="h-7 text-xs"
                    />
                    <p className="text-[0.6875rem] text-muted-foreground">
                      {tagMembers.length > 0 ? `成员：${tagMembers.map((entry) => entry.node.name).join("、")}` : "暂无成员（空标签有效，规则不匹配任何节点）"}
                      {blocker ? ` · ${blocker}` : ""}
                    </p>
                    {issues.map((issue, position) => (
                      <p key={position} className="text-xs text-destructive">
                        {issue.message}
                      </p>
                    ))}
                  </li>
                )
              })}
            </ul>
          )}
          <form
            className="grid gap-2 border-t px-4 py-3 sm:grid-cols-[1fr_1.4fr_auto] sm:items-end"
            onSubmit={(event) => {
              event.preventDefault()
              addTag()
            }}
          >
            <div className="flex flex-col gap-1">
              <Label htmlFor={nameId} className="text-xs">
                名称
              </Label>
              <Input
                id={nameId}
                data-testid="tag-new-name"
                value={newName}
                placeholder="edge"
                onChange={(event) => setNewName(event.target.value)}
                aria-invalid={candidateIssues.length > 0 || undefined}
                className="font-mono"
                disabled={pending}
              />
            </div>
            <div className="flex flex-col gap-1">
              <Label htmlFor={descriptionId} className="text-xs">
                说明
              </Label>
              <Input
                id={descriptionId}
                value={newDescription}
                placeholder="可选"
                maxLength={TAG_LIMITS.description}
                onChange={(event) => setNewDescription(event.target.value)}
                disabled={pending}
              />
            </div>
            <Button type="submit" variant="outline" data-testid="tag-add" disabled={pending || newName.trim() === "" || candidateIssues.length > 0}>
              <PlusIcon data-icon="inline-start" />
              添加
            </Button>
            {newName.trim() !== "" ? (
              <p className={cn("text-xs sm:col-span-3", candidateIssues.length > 0 ? "text-destructive" : "text-muted-foreground")}>
                {candidateIssues[0]?.message ?? `将添加 ${candidate}`}
              </p>
            ) : null}
          </form>
          <div className="flex items-center gap-2 border-t px-4 py-3">
            <span className="min-w-0 flex-1 text-xs text-muted-foreground">
              {catalogDirty ? "标签目录有未保存的修改。" : `与已保存的 r${snapshot.revision} 一致。`}
            </span>
            {catalogDirty ? (
              <Button variant="ghost" size="sm" disabled={pending} onClick={() => setCatalog(snapshot.tags)}>
                <RotateCcwIcon data-icon="inline-start" />
                放弃
              </Button>
            ) : null}
            <Button
              size="sm"
              data-testid="tag-catalog-save"
              disabled={pending || !catalogDirty || catalogIssues.length > 0}
              onClick={saveCatalog}
              title={`条件保存：仅当 controller 仍为 r${snapshot.revision} 时写入`}
            >
              {pending ? <Spinner data-icon="inline-start" role="presentation" aria-hidden="true" /> : <SaveIcon data-icon="inline-start" />}
              保存标签目录
            </Button>
          </div>
        </Panel>

        <Panel
          data-testid="tag-assignments"
          title="节点标签"
          description="为每个已注册节点选择所属标签。"
          bodyClassName="flex flex-col"
        >
          {active.length === 0 ? (
            <p className="px-4 py-6 text-sm text-muted-foreground">没有已注册的活动节点。</p>
          ) : (
            <ul aria-label="节点标签" className="divide-y">
              {active.map((entry) => {
                const current = tagsOfNode(snapshot, entry.node.id)
                const editing = editingNode === entry.node.id
                return (
                  <li
                    key={entry.node.id}
                    data-testid="node-tags-row"
                    className={cn("flex flex-col gap-2 px-4 py-3", editing && "bg-muted/30")}
                  >
                    <div className="flex items-center gap-2">
                      <Link href={routes.machine(entry.node.id)} className="text-sm font-medium hover:underline">
                        {entry.node.name}
                      </Link>
                      <span className="font-mono text-[0.6875rem] text-muted-foreground" title={entry.node.id}>
                        {entry.node.mesh_ipv4} · {entry.node.id.slice(0, 8)}
                      </span>
                      {!editing ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="ml-auto"
                          aria-label={`编辑 ${entry.node.name} 的标签`}
                          disabled={pending || (editingNode !== null && !editing) || snapshot.tags.length === 0}
                          onClick={() => startNodeEdit(entry.node.id)}
                        >
                          <PencilIcon data-icon="inline-start" />
                          编辑
                        </Button>
                      ) : null}
                    </div>
                    {editing ? (
                      <div className="flex flex-col gap-2">
                        <div className="flex flex-wrap gap-x-4 gap-y-2">
                          {snapshot.tags.map((definition) => {
                            const checkboxId = `${entry.node.id}-${definition.tag}`
                            return (
                              <div key={definition.tag} className="flex items-center gap-1.5">
                                <Checkbox
                                  id={checkboxId}
                                  checked={nodeDraft.includes(definition.tag)}
                                  onCheckedChange={(checked) =>
                                    setNodeDraft((draft) =>
                                      checked === true ? [...draft.filter((tag) => tag !== definition.tag), definition.tag] : draft.filter((tag) => tag !== definition.tag)
                                    )
                                  }
                                />
                                <Label htmlFor={checkboxId} className="font-mono text-xs font-normal">
                                  {definition.tag.slice(4)}
                                </Label>
                              </div>
                            )
                          })}
                        </div>
                        <div className="flex items-center gap-2">
                          <span className="min-w-0 flex-1 text-[0.6875rem] text-muted-foreground">
                            只能选择已保存的标签；新标签先保存目录。
                          </span>
                          <Button variant="ghost" size="sm" disabled={pending} onClick={() => setEditingNode(null)}>
                            取消
                          </Button>
                          <Button
                            size="sm"
                            data-testid="node-tags-save"
                            disabled={pending || nodeDraft.length > TAG_LIMITS.perNode}
                            onClick={() => saveNode(entry.node.id)}
                          >
                            保存
                          </Button>
                        </div>
                      </div>
                    ) : current.length > 0 ? (
                      <ul aria-label={`${entry.node.name} 的标签`} className="flex flex-wrap gap-1">
                        {current.map((tag) => (
                          <li key={tag}>
                            <Badge variant="outline" className="font-mono">
                              {tag.slice(4)}
                            </Badge>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <p className="text-xs text-muted-foreground">没有标签</p>
                    )}
                  </li>
                )
              })}
            </ul>
          )}
        </Panel>
      </div>
    </div>
  )
}

function FeedbackLine({ feedback, onReload }: { feedback: Feedback; onReload: () => void }) {
  if (!feedback) return null
  const { result } = feedback
  if (result.status === "saved") {
    return (
      <p role="status" data-testid="tags-save-status" className="text-xs text-success">
        已保存为 r{result.snapshot.revision}。
      </p>
    )
  }
  if (result.status === "conflict") {
    return (
      <Alert variant="destructive" data-testid="tags-save-status">
        <CircleAlertIcon />
        <AlertTitle>版本冲突，未保存</AlertTitle>
        <AlertDescription className="flex flex-col items-start gap-2">
          <p>{result.message}</p>
          <Button variant="outline" size="sm" onClick={onReload}>
            重新读取
          </Button>
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <p role="alert" data-testid="tags-save-status" className="flex items-start gap-1.5 text-xs text-destructive">
      <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
      <span className="break-all">{result.message}</span>
    </p>
  )
}
