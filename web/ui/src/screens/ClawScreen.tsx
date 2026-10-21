import { useCallback, useState } from 'react'
import { ChevronDownIcon, ChevronUpIcon, Loader2Icon } from 'lucide-react'

import { ApiError, api } from '@/api/client'
import type { BearclawResponse, ClawStatus, Session } from '@/api/types'
import { TabBar } from '@/components/TabBar'
import { Button } from '@/components/ui/button'
import { useBearclaw } from '@/hooks/useBearclaw'
import { usePersistentState } from '@/hooks/usePersistentState'
import { usePoller } from '@/hooks/usePoller'
import { usePrs } from '@/hooks/usePrs'
import { useSettings } from '@/hooks/useSettings'
import { useUsage } from '@/hooks/useUsage'
import { clawStats, type ClawStats } from '@/lib/claw'
import { SessionScreen } from '@/screens/SessionScreen'

const parseOpen = (v: string) => v !== 'false'

/**
 * `#/claw`: Claw, the operator session the terminal dashboard runs (its Claw tab) — its chat with
 * a composer, under the dashboard's character card. Not running: one tap starts it (/api/claw).
 */
export function ClawScreen() {
  const settings = useSettings()
  const { bearclaw, refresh } = useBearclaw()
  const [status, setStatus] = useState<ClawStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [starting, setStarting] = useState(false)

  // The row comes with the bearclaw (stamped `claw`); /api/claw only answers where it lives.
  const fromList = findClaw(bearclaw)
  const session = fromList ?? status?.session ?? null

  usePoller(
    async (signal) => {
      try {
        setStatus(await api.claw({ signal }))
        setError(null)
      } catch (err) {
        if ((err as Error)?.name === 'AbortError') return
        setError(err instanceof ApiError && err.status === 404 ? 'This server predates Claw: update bearclaw and restart its web server.' : (err as Error)?.message || 'could not reach Claw')
      }
    },
    starting ? 2000 : 10_000,
    { enabled: !fromList && settings.loaded },
  )

  const start = useCallback(async () => {
    setStarting(true)
    setError(null)
    try {
      setStatus(await api.startClaw())
      refresh()
    } catch (err) {
      setError((err as Error)?.message || 'could not start Claw')
      setStarting(false)
    }
  }, [refresh])

  const chrome = (
    <div className="shrink-0 border-b bg-background pt-safe px-safe">
      <div className="mx-auto w-full max-w-3xl px-3 pt-2.5">
        <TabBar />
        <ClawCard />
      </div>
    </div>
  )

  if (session) return <SessionScreen key={`${session.host}/${session.session_id}`} host={session.host} id={session.session_id} chrome={chrome} />

  return (
    <div className="fixed-app flex flex-col overflow-hidden bg-background">
      {chrome}
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center font-mono">
        <p className="text-sm text-muted-foreground">
          {starting ? 'Claw is starting…' : 'Claw is not running. It is the Claude session that runs bearclaw for you: ask it what everyone is doing, tell it to message a session.'}
        </p>
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
        <Button onClick={() => void start()} disabled={starting || !settings.claw}>
          {starting ? <Loader2Icon className="animate-spin" /> : null} Start Claw
        </Button>
      </div>
    </div>
  )
}

/** Claw's row in the bearclaw (the server stamps it `claw`), or null. */
function findClaw(bearclaw: BearclawResponse | null): Session | null {
  for (const h of bearclaw?.hosts ?? []) for (const s of h.sessions ?? []) if (s.claw) return { ...s, host: s.host ?? h.name }
  return null
}

const ORANGE = '#ff7c0a'
const STRIP = ['#0b0806', '#3a2516', '#5e3c24', '#8a5a36', '#c08a5c', '#6e6b68', '#b9b4ab', '#f4efe4', ORANGE]

/** The dashboard's character card: the druid portrait and the numbers that matter right now. */
function ClawCard() {
  const [open, setOpen] = usePersistentState<boolean>('bearclaw.clawCard', true, parseOpen)
  const stats = useClawStats()
  const lines = clawLines(stats)
  return (
    <section aria-label="Claw" className="pb-2 font-mono text-[0.8125rem] leading-[1.35rem]">
      <div className="flex gap-3">
        {open ? (
          <img src="/icon.svg" alt="" width={88} height={88} className="size-[88px] shrink-0 rounded-sm [image-rendering:pixelated]" />
        ) : null}
        <div className="min-w-0 flex-1">
          <button type="button" onClick={() => setOpen(!open)} aria-expanded={open} className="flex w-full items-center gap-1 text-left font-bold" style={{ color: ORANGE }}>
            druid@bearclaw
            {open ? <ChevronUpIcon className="size-4 text-dimmer" /> : <ChevronDownIcon className="size-4 text-dimmer" />}
            {!open ? <span className="ml-1 truncate font-normal text-muted-foreground">{lines[1]?.[1]}</span> : null}
          </button>
          {open ? (
            <dl>
              {lines.map(([k, v]) => (
                <div key={k} className="flex min-w-0 gap-1.5">
                  <dt className="shrink-0 font-bold" style={{ color: ORANGE }}>
                    {k}:
                  </dt>
                  <dd className="min-w-0 truncate">{v}</dd>
                </div>
              ))}
            </dl>
          ) : null}
        </div>
      </div>
      {open ? (
        <div aria-hidden className="mt-2 flex h-2.5">
          {STRIP.map((c) => (
            <span key={c} className="flex-1" style={{ background: c }} />
          ))}
        </div>
      ) : null}
    </section>
  )
}

function clawLines(s: ClawStats): [string, string][] {
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`
  const waiting = s.prWaiting == null ? plural(s.waiting, 'session', 'sessions') : `${plural(s.waiting, 'session', 'sessions')} · ${plural(s.prWaiting, 'PR', 'PRs')}`
  return [
    ['Sessions', `${s.sessions} (${s.busy} busy, ${s.idle} idle)`],
    ['Waiting on you', waiting],
    ['Todo', s.todo == null ? '—' : `${s.todo} in progress`],
    ['Usage', s.usage ?? '—'],
    ['Memory', s.brain == null ? '—' : plural(s.brain, 'note', 'notes')],
  ]
}

/** What the card shows, from what the app already loads plus the todo and the brain (every 30 s). */
function useClawStats(): ClawStats {
  const { bearclaw } = useBearclaw()
  const prs = usePrs()
  const { accounts } = useUsage()
  const { self } = useSettings()
  const [extra, setExtra] = useState<{ todo: number | null; brain: number | null }>({ todo: null, brain: null })
  usePoller(
    async (signal) => {
      if (!self) return
      const [todo, brain] = await Promise.all([
        api.todo(self, { signal }).catch(() => null),
        api.notesTree(self, { signal, book: 'brain' }).catch(() => null),
      ])
      setExtra({ todo: todo ? todo.items.filter((i) => i.status === 'doing').length : null, brain: brain ? brain.files.filter((f) => f.kind === 'markdown').length : null })
    },
    30_000,
    { enabled: !!self },
  )
  return clawStats({ bearclaw, prs: prs.data, usage: accounts[0]?.usage ?? null, ...extra })
}

