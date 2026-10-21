import { useMemo, useState } from 'react'
import { CalendarIcon, FolderKanbanIcon, ListTodoIcon, RefreshCwIcon, SearchIcon, TriangleAlertIcon } from 'lucide-react'

import { ApiError, api } from '@/api/client'
import type { ProductiveTask, TasksResponse } from '@/api/types'
import { ScreenHeader } from '@/components/ScreenHeader'
import { TaskDialog } from '@/components/TaskDialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { useNow } from '@/hooks/useNow'
import { usePersistentState } from '@/hooks/usePersistentState'
import { usePoller } from '@/hooks/usePoller'
import { useSettings } from '@/hooks/useSettings'
import { relTime } from '@/lib/format'
import { dueTone, filterTasks, groupTasks, localDay, type TaskGrouping } from '@/lib/tasks'
import { cn } from '@/lib/utils'

const parseGrouping = (v: string): TaskGrouping | undefined => (v === 'status' || v === 'project' ? v : undefined)

const DUE_TONE = {
  overdue: { border: 'border-l-[3px] border-l-status-error', text: 'text-status-error' },
  soon: { border: 'border-l-[3px] border-l-status-waiting', text: 'text-status-waiting' },
} as const

/**
 * `#/tasks`: your open Productive tasks (`bearclaw tasks`) laid out like the PRs screen — a section
 * per status (or project), a card per task, a filter; overdue and due-soon tasks marked. A card
 * opens the task: its description, Productive, a session on it, the todo.
 */
export function TasksScreen() {
  const { self } = useSettings()
  const now = useNow(30_000)
  const [data, setData] = useState<(TasksResponse & { fetchedAt: number }) | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [q, setQ] = useState('')
  const [open, setOpen] = useState<ProductiveTask | null>(null)
  const [grouping, setGrouping] = usePersistentState<TaskGrouping>('bearclaw.tasksGroup', 'status', parseGrouping)

  const reload = usePoller(
    async (signal) => {
      if (!self) return
      setLoading(true)
      try {
        setData({ ...(await api.tasks(self, { signal })), fetchedAt: Date.now() })
        setError(null)
      } catch (err) {
        if ((err as Error)?.name === 'AbortError') return
        setError(err instanceof ApiError && (err.status === 404 || err.status === 501) ? 'This server predates the Tasks tab: update bearclaw and restart its web server.' : (err as Error)?.message || 'could not load tasks')
      } finally {
        setLoading(false)
      }
    },
    5 * 60_000,
    { enabled: !!self },
  )

  const shown = useMemo(() => filterTasks(data?.tasks ?? [], q), [data, q])
  const groups = useMemo(() => groupTasks(shown, grouping), [shown, grouping])
  const today = localDay(new Date(now))

  const byProject = (
    <button
      type="button"
      aria-pressed={grouping === 'project'}
      onClick={() => setGrouping(grouping === 'project' ? 'status' : 'project')}
      title={grouping === 'project' ? 'Group by status' : 'Group by project'}
      className={cn(
        'flex h-9 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/50',
        grouping === 'project' ? 'border-primary/40 bg-primary/10 text-foreground' : 'text-muted-foreground hover:bg-muted/60',
      )}
    >
      <FolderKanbanIcon className="size-3.5" />
      Projects
    </button>
  )

  return (
    <div className="flex min-h-app flex-col bg-background">
      <ScreenHeader>
        <div className="flex min-h-8 items-center gap-1">
          <h1 className="flex-1 text-xl font-bold tracking-tight">Tasks</h1>
          <Button
            variant="ghost"
            size="icon"
            aria-label="Refresh tasks"
            title="Fetch from Productive now"
            onClick={reload}
            disabled={loading || !self}
            className="-my-1.5 size-11 shrink-0 rounded-xl"
          >
            <RefreshCwIcon className={cn('size-5', loading && 'animate-spin')} />
          </Button>
        </div>
        <div className="mt-2 flex items-center gap-2">
          <div className="relative w-full">
            <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Filter by title, project, #123" aria-label="Filter tasks" className="h-8 pl-8" />
          </div>
          {byProject}
        </div>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-lg space-y-5 px-4 pt-3 pb-[max(1.5rem,env(safe-area-inset-bottom))] md:max-w-2xl">
        <p className={cn('text-xs', error ? 'text-status-waiting' : 'text-dimmer')}>
          {error ? (
            <span className="inline-flex items-start gap-1">
              <TriangleAlertIcon className="mt-px size-3.5 shrink-0" />
              {error}
            </span>
          ) : data ? (
            `Fetched ${relTime(data.fetchedAt, now) || 'just now'}${relTime(data.fetchedAt, now) ? ' ago' : ''} · ${data.tasks.length} open`
          ) : null}
        </p>
        {!data && !error ? (
          <div className="space-y-2">
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} className="h-20 rounded-lg" />
            ))}
          </div>
        ) : null}
        {groups.map((g) => (
          <section key={g.label} aria-label={g.label}>
            <h2 className="mb-2 flex items-baseline gap-2 text-sm font-semibold">
              {g.label}
              <span className="text-xs font-normal text-dimmer tabular-nums">{g.tasks.length}</span>
            </h2>
            <div className="space-y-2">
              {g.tasks.map((t) => (
                <TaskCard key={t.id} task={t} tone={dueTone(t.dueDate, today)} grouping={grouping} onOpen={setOpen} />
              ))}
            </div>
          </section>
        ))}
        {data && !groups.length ? <p className="py-8 text-center text-sm text-muted-foreground">{q ? 'No task matches.' : 'No open tasks.'}</p> : null}
      </div>
      {open && self ? <TaskDialog key={open.id} task={open} host={self} onClose={() => setOpen(null)} /> : null}
    </div>
  )
}

function TaskCard({ task: t, tone, grouping, onOpen }: { task: ProductiveTask; tone: ReturnType<typeof dueTone>; grouping: TaskGrouping; onOpen: (t: ProductiveTask) => void }) {
  const toneCls = tone ? DUE_TONE[tone] : null
  return (
    <div className={cn('rounded-lg border bg-card text-sm hover:border-primary/40', toneCls?.border)}>
      <button type="button" onClick={() => onOpen(t)} className="block w-full rounded-lg p-3 text-left outline-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring/50">
        <div className="mb-1 flex items-center gap-1.5 text-xs text-muted-foreground">
          <ListTodoIcon className="size-3.5 shrink-0" />
          <span className="shrink-0 font-mono">#{t.number}</span>
          <span className="min-w-0 truncate">{grouping === 'status' ? t.project : t.status}</span>
          {t.dueDate ? (
            <span className={cn('ml-auto inline-flex shrink-0 items-center gap-1 tabular-nums', toneCls?.text)} title={tone === 'overdue' ? 'Overdue' : 'Due'}>
              <CalendarIcon className="size-3" />
              {t.dueDate.slice(5)}
            </span>
          ) : null}
        </div>
        <div className="line-clamp-2 font-medium text-foreground">{t.title}</div>
        {t.description ? <div className="mt-1.5 line-clamp-1 text-xs text-muted-foreground">{t.description}</div> : null}
      </button>
    </div>
  )
}
