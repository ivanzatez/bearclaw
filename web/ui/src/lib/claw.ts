// The Claw tab's character card (screens/ClawScreen.tsx): the terminal dashboard's numbers, from
// what the app loads. Pure, unit-tested in claw.test.ts.
import type { BearclawResponse, PrsResponse, UsageResponse } from '@/api/types'
import { allSessions, statusMeta } from '@/lib/sessions'

export interface ClawStats {
  sessions: number
  busy: number
  idle: number
  waiting: number
  /** PRs in "Waiting on me" (null: not loaded). */
  prWaiting: number | null
  todo: number | null
  /** `34% session · 61% week` (null: not loaded). */
  usage: string | null
  brain: number | null
}

export function clawStats({
  bearclaw,
  prs,
  usage,
  todo,
  brain,
}: {
  bearclaw: BearclawResponse | null
  prs: PrsResponse | null
  usage: UsageResponse | null
  todo: number | null
  brain: number | null
}): ClawStats {
  const out = { sessions: 0, busy: 0, idle: 0, waiting: 0 }
  for (const s of allSessions(bearclaw)) {
    out.sessions += 1
    const k = statusMeta(s.status).key
    if (k === 'busy' || k === 'idle' || k === 'waiting') out[k] += 1
  }
  const pct = (group: string) => usage?.limits.find((l) => l.group === group && l.active !== false)?.percent ?? usage?.limits.find((l) => l.group === group)?.percent
  const session = pct('session')
  const week = pct('weekly')
  const parts = [session != null ? `${Math.round(session)}% session` : null, week != null ? `${Math.round(week)}% week` : null].filter(Boolean)
  return {
    ...out,
    prWaiting: prs ? prs.prs.filter((p) => p.column === 'waitingOnMe').length : null,
    todo,
    usage: parts.length ? parts.join(' · ') : null,
    brain,
  }
}
