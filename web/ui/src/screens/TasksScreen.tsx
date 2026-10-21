import { useMemo, useState } from 'react'
import { ExternalLinkIcon } from 'lucide-react'

import { ApiError, api } from '@/api/client'
import type { TasksResponse } from '@/api/types'
import { ScreenHeader } from '@/components/ScreenHeader'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { usePersistentState } from '@/hooks/usePersistentState'
import { usePoller } from '@/hooks/usePoller'
import { useSettings } from '@/hooks/useSettings'
import { groupTasks, type TaskGrouping } from '@/lib/tasks'
import { cn } from '@/lib/utils'

const parseGrouping = (v: string): TaskGrouping | undefined => (v === 'status' || v === 'project' ? v : undefined)

/**
 * `#/tasks`: your open Productive tasks (`bearclaw tasks`), grouped by status or project like the
 * dashboard's Tasks tab; a tap opens the task in Productive.
 */
export function TasksScreen() {
  const { self } = useSettings()
  const [data, setData] = useState<TasksResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [grouping, setGrouping] = usePersistentState<TaskGrouping>('bearclaw.tasksGroup', 'status', parseGrouping)

  usePoller(
    async (signal) => {
      if (!self) return
      try {
        setData(await api.tasks(self, { signal }))
        setError(null)
      } catch (err) {
        if ((err as Error)?.name === 'AbortError') return
        setError(err instanceof ApiError && (err.status === 404 || err.status === 501) ? 'This server predates the Tasks tab: update bearclaw and restart its web server.' : (err as Error)?.message || 'could not load tasks')
      }
    },
    5 * 60_000,
    { enabled: !!self },
  )

  const groups = useMemo(() => groupTasks(data?.tasks ?? [], grouping), [data, grouping])

  return (
    <div className="flex min-h-app flex-col bg-background">
      <ScreenHeader>
        <div className="flex min-h-8 items-center gap-2">
          <h1 className="text-xl font-bold tracking-tight">Tasks</h1>
          <span className="flex-1 text-xs text-dimmer tabular-nums">{data ? `${data.tasks.length} open` : ''}</span>
          <ToggleGroup type="single" value={grouping} onValueChange={(v) => v && setGrouping(v as TaskGrouping)} spacing={0} aria-label="Group by" className="rounded-md bg-muted p-0.5">
            <ToggleGroupItem value="status" className="h-7 px-2 text-xs data-[state=on]:bg-background">
              status
            </ToggleGroupItem>
            <ToggleGroupItem value="project" className="h-7 px-2 text-xs data-[state=on]:bg-background">
              project
            </ToggleGroupItem>
          </ToggleGroup>
        </div>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-3xl px-3 pb-[max(1.5rem,env(safe-area-inset-bottom))] font-mono text-[0.875rem]">
        {error ? <p className="py-6 text-sm text-destructive">{error}</p> : null}
        {!data && !error ? <p className="py-6 text-sm text-dimmer">Loading your Productive tasks…</p> : null}
        {groups.map((g) => (
          <section key={g.label} aria-label={g.label} className="pt-3">
            <h2 className="flex items-baseline gap-2 border-b pb-1 text-[0.8125rem] font-bold text-primary">
              {g.label} <span className="font-normal text-dimmer">({g.tasks.length})</span>
            </h2>
            {g.tasks.map((t) => (
              <a key={t.id} href={t.url} target="_blank" rel="noreferrer" className="flex items-start gap-2 border-b border-border/60 py-2.5 active:bg-muted/60">
                <span className="w-16 shrink-0 text-xs text-dimmer tabular-nums">#{t.number}</span>
                <span className="min-w-0 flex-1">
                  <span className="block [overflow-wrap:anywhere]">{t.title}</span>
                  <span className={cn('mt-0.5 block truncate text-xs', 'text-dimmer')}>
                    {grouping === 'status' ? t.project : t.status}
                    {t.dueDate ? <span className="text-status-waiting"> · due {t.dueDate}</span> : null}
                  </span>
                </span>
                <ExternalLinkIcon className="mt-1 size-3.5 shrink-0 text-dimmer" />
              </a>
            ))}
          </section>
        ))}
        {data && !data.tasks.length ? <p className="py-6 text-sm text-dimmer">No open tasks.</p> : null}
      </div>
    </div>
  )
}
