import { useState } from 'react'
import { ExternalLinkIcon, ListPlusIcon, Loader2Icon, PlayIcon } from 'lucide-react'
import { toast } from 'sonner'
import { useLocation } from 'wouter'

import { ApiError, api } from '@/api/client'
import type { ProductiveTask } from '@/api/types'
import { watchForSpawned } from '@/api/spawnWatch'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { useBearclaw } from '@/hooks/useBearclaw'
import { shortCwd } from '@/lib/format'
import { nextReviewName } from '@/lib/prReview'
import { allSessions, sessionHref } from '@/lib/sessions'
import { taskPromptFor } from '@/lib/tasks'

/**
 * One Productive task (the Tasks tab): its description, open it in Productive, put it on the todo,
 * or start a session on it in the first start directory, told the task and what to do.
 */
export function TaskDialog({ task: t, host, onClose }: { task: ProductiveTask; host: string; onClose: () => void }) {
  const [, navigate] = useLocation()
  const { bearclaw, applyBearclaw } = useBearclaw()
  const [prompt, setPrompt] = useState('')
  const [busy, setBusy] = useState<null | 'spawn' | 'todo'>(null)
  const [error, setError] = useState<string | null>(null)
  const dir = bearclaw?.hosts.find((h) => h.name === host)?.spawnDirs?.[0]?.path ?? null

  const spawn = async () => {
    if (busy) return
    setBusy('spawn')
    setError(null)
    const taken = new Set(allSessions(bearclaw).map((s) => s.tmux_session ?? ''))
    let name = nextReviewName(`task-${t.number}`, taken)
    try {
      let res
      for (let tries = 0; ; tries++) {
        try {
          res = await api.spawn(host, { name, dir: dir ?? undefined, prompt: taskPromptFor(t, prompt) })
          break
        } catch (err) {
          if (!(err instanceof ApiError && err.status === 409) || tries >= 5) throw err
          taken.add(name)
          name = nextReviewName(`task-${t.number}`, taken)
        }
      }
      onClose()
      const id = toast.loading(`Starting ${name}…`, { description: dir ? shortCwd(dir) : undefined })
      watchForSpawned(
        { host: res.host || host, name, tmuxSession: res.tmuxSession ?? name },
        {
          onBearclaw: applyBearclaw,
          onFound: (s) => {
            toast.success(`${name} is up`, { id, description: undefined })
            navigate(sessionHref(s))
          },
          onTimeout: () => toast.error(`${name} has not shown up yet`, { id, description: 'Check the list in a moment' }),
        },
      )
    } catch (err) {
      setError((err as Error)?.message || 'Failed to start')
      setBusy(null)
    }
  }

  const addTodo = async () => {
    setBusy('todo')
    setError(null)
    try {
      await api.editTodo(host, { op: 'add', text: `#${t.number} ${t.title}`.slice(0, 300), task: t.url })
      toast.success('On the todo')
      setBusy(null)
    } catch (err) {
      setError((err as Error)?.message || 'Not added')
      setBusy(null)
    }
  }

  return (
    <Dialog open onOpenChange={(o) => (o ? null : onClose())}>
      <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogDescription className="font-mono text-xs">
            #{t.number} · {t.project}
          </DialogDescription>
          <DialogTitle className="text-base leading-snug">{t.title}</DialogTitle>
        </DialogHeader>

        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
          <dt className="text-muted-foreground">Status</dt>
          <dd>{t.status}</dd>
          {t.dueDate ? (
            <>
              <dt className="text-muted-foreground">Due</dt>
              <dd className="tabular-nums">{t.dueDate}</dd>
            </>
          ) : null}
        </dl>

        {t.description ? <p className="max-h-60 overflow-y-auto rounded-md border bg-muted/30 p-2.5 text-xs whitespace-pre-wrap text-muted-foreground">{t.description}</p> : null}

        <div className="space-y-1.5">
          <Label htmlFor="task-prompt">Start a session on it</Label>
          <Textarea id="task-prompt" value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={3} placeholder="What should it do? (optional)" />
          <p className="text-xs text-dimmer">Runs in {dir ? shortCwd(dir) : 'the default directory'}; the session is told the task and its link.</p>
        </div>

        {error ? (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}

        <div className="flex flex-wrap items-center gap-2">
          <Button asChild variant="ghost" size="sm" className="text-muted-foreground">
            <a href={t.url} target="_blank" rel="noreferrer">
              <ExternalLinkIcon /> Productive
            </a>
          </Button>
          <Button variant="outline" size="sm" onClick={() => void addTodo()} disabled={!!busy}>
            {busy === 'todo' ? <Loader2Icon className="animate-spin" /> : <ListPlusIcon />} Todo
          </Button>
          <Button size="sm" className="ml-auto" onClick={() => void spawn()} disabled={!!busy}>
            {busy === 'spawn' ? <Loader2Icon className="animate-spin" /> : <PlayIcon />} Start session
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
