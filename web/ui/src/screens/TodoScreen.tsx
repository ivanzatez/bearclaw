import { useState, type FormEvent } from 'react'
import { CheckIcon, ExternalLinkIcon, Loader2Icon, PlusIcon, Trash2Icon } from 'lucide-react'
import { toast } from 'sonner'

import { api } from '@/api/client'
import type { TodoEdit, TodoItem, TodoResponse } from '@/api/types'
import { ScreenHeader } from '@/components/ScreenHeader'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useBearclaw } from '@/hooks/useBearclaw'
import { usePoller } from '@/hooks/usePoller'
import { useSettings } from '@/hooks/useSettings'
import { relTime } from '@/lib/format'
import { allSessions, sessionHref } from '@/lib/sessions'
import { todoGroups } from '@/lib/todo'
import { cn } from '@/lib/utils'

const MARK: Record<TodoItem['status'], string> = { doing: '●', open: '○', done: '✓' }

/**
 * `#/todo`: the one-line work list every session keeps current (`bearclaw todo`) — ● doing, with
 * the session it is on, ○ open, ✓ done. Add an item, mark one done, remove one.
 */
export function TodoScreen() {
  const { self } = useSettings()
  const { bearclaw } = useBearclaw()
  const [data, setData] = useState<TodoResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [text, setText] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [showDone, setShowDone] = useState(false)

  usePoller(
    async (signal) => {
      if (!self) return
      try {
        setData(await api.todo(self, { signal }))
        setError(null)
      } catch (err) {
        if ((err as Error)?.name !== 'AbortError') setError((err as Error)?.message || 'could not load the todo')
      }
    },
    10_000,
    { enabled: !!self },
  )

  const run = async (key: string, edit: TodoEdit) => {
    if (!self) return
    setBusy(key)
    try {
      setData(await api.editTodo(self, edit))
      return true
    } catch (err) {
      toast.error('Not changed', { description: (err as Error)?.message })
      return false
    } finally {
      setBusy(null)
    }
  }
  const add = async (e: FormEvent) => {
    e.preventDefault()
    const line = text.trim()
    if (line && (await run('add', { op: 'add', text: line }))) setText('')
  }

  const live = new Map(allSessions(bearclaw).map((s) => [s.session_id, s]))
  const groups = todoGroups(data?.items ?? [])

  return (
    <div className="flex min-h-app flex-col bg-background">
      <ScreenHeader>
        <div className="flex min-h-8 items-baseline gap-2">
          <h1 className="text-xl font-bold tracking-tight">Todo</h1>
          <span className="text-xs text-dimmer tabular-nums">
            {groups.doing.length} doing · {groups.open.length} open
          </span>
        </div>
        <form onSubmit={add} className="mt-2 flex gap-2">
          <Input value={text} onChange={(e) => setText(e.target.value)} placeholder="Add an item nobody is on yet…" aria-label="New todo item" enterKeyHint="done" className="h-10 text-base" />
          <Button type="submit" size="icon" aria-label="Add" disabled={!text.trim() || busy === 'add'} className="size-10 shrink-0">
            {busy === 'add' ? <Loader2Icon className="animate-spin" /> : <PlusIcon />}
          </Button>
        </form>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-3xl px-3 pb-[max(1.5rem,env(safe-area-inset-bottom))] font-mono text-[0.875rem]">
        {error && !data ? <p className="py-6 text-sm text-destructive">{error}</p> : null}
        {!data && !error ? <p className="py-6 text-sm text-dimmer">Loading…</p> : null}
        {[...groups.doing, ...groups.open, ...(showDone ? groups.done : [])].map((item) => {
          const session = item.session ? live.get(item.session) : undefined
          return (
            <div key={item.id} className="flex items-start gap-2 border-b border-border/60 py-2.5">
              <span className={cn('w-4 shrink-0 text-center', item.status === 'doing' ? 'text-status-busy' : item.status === 'done' ? 'text-dimmer' : 'text-muted-foreground')}>{MARK[item.status]}</span>
              <div className="min-w-0 flex-1">
                <div className={cn('[overflow-wrap:anywhere]', item.status === 'done' && 'text-dimmer line-through')}>
                  <span className="mr-1.5 text-dimmer">#{item.id}</span>
                  {item.text}
                </div>
                <div className="mt-0.5 flex flex-wrap gap-x-3 text-xs text-dimmer">
                  {session ? (
                    <a href={`#${sessionHref(session)}`} className="text-primary">
                      ⧉ {session.display_title || session.name}
                    </a>
                  ) : item.sessionLabel && item.status === 'doing' ? (
                    <span>⧉ {item.sessionLabel} (gone)</span>
                  ) : null}
                  {item.taskUrl ? (
                    <a href={item.taskUrl} target="_blank" rel="noreferrer" className="inline-flex items-center gap-0.5 text-primary">
                      task <ExternalLinkIcon className="size-3" />
                    </a>
                  ) : null}
                  <span>{relTime(item.updatedMs)} ago</span>
                </div>
              </div>
              {item.status !== 'done' ? (
                <Button variant="ghost" size="icon" aria-label={`Mark #${item.id} done`} disabled={!!busy} onClick={() => void run(`done${item.id}`, { op: 'done', id: item.id })} className="size-9 shrink-0 text-muted-foreground">
                  {busy === `done${item.id}` ? <Loader2Icon className="animate-spin" /> : <CheckIcon />}
                </Button>
              ) : null}
              <Button
                variant="ghost"
                size="icon"
                aria-label={`Remove #${item.id}`}
                disabled={!!busy}
                onClick={() => {
                  if (window.confirm(`Remove #${item.id}: ${item.text}?`)) void run(`rm${item.id}`, { op: 'rm', id: item.id })
                }}
                className="size-9 shrink-0 text-dimmer"
              >
                {busy === `rm${item.id}` ? <Loader2Icon className="animate-spin" /> : <Trash2Icon />}
              </Button>
            </div>
          )
        })}
        {data && !groups.doing.length && !groups.open.length ? <p className="py-6 text-sm text-dimmer">Nothing open.</p> : null}
        {groups.done.length ? (
          <button type="button" onClick={() => setShowDone(!showDone)} className="py-3 text-xs text-dimmer">
            {showDone ? 'Hide' : 'Show'} {groups.done.length} done
          </button>
        ) : null}
      </div>
    </div>
  )
}
