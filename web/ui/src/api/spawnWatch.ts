// After a spawn: poll /api/bearclaw until Claude registers the new session, then hand it
// over (the caller navigates to it). One watch at a time; a new spawn replaces the old one.
import { api } from '@/api/client'
import type { BearclawResponse, Session, SpawnResponse } from '@/api/types'
import { findSpawned } from '@/lib/sessions'

export interface SpawnWatchHandlers {
  /** Every fresh bearclaw body (so the list updates too). */
  onBearclaw: (bearclaw: BearclawResponse) => void
  onFound: (session: Session) => void
  onTimeout: () => void
}

let cancelCurrent: (() => void) | null = null

export function watchForSpawned(
  spawned: Pick<SpawnResponse, 'host' | 'name' | 'tmuxSession'>,
  handlers: SpawnWatchHandlers,
  { intervalMs = 1500, timeoutMs = 45_000 } = {},
): () => void {
  cancelCurrent?.()
  const deadline = Date.now() + timeoutMs
  let timer: ReturnType<typeof setTimeout> | undefined
  let stopped = false
  const ctl = new AbortController()

  const tick = async () => {
    try {
      const bearclaw = await api.bearclaw({ signal: ctl.signal })
      if (stopped) return
      handlers.onBearclaw(bearclaw)
      const found = findSpawned(bearclaw, spawned.host, spawned)
      if (found) {
        stop()
        handlers.onFound(found)
        return
      }
    } catch {
      if (stopped) return // keep trying otherwise
    }
    if (Date.now() > deadline) {
      stop()
      handlers.onTimeout()
      return
    }
    timer = setTimeout(tick, intervalMs)
  }

  function stop() {
    stopped = true
    clearTimeout(timer)
    ctl.abort()
    if (cancelCurrent === stop) cancelCurrent = null
  }

  cancelCurrent = stop
  timer = setTimeout(tick, intervalMs)
  return stop
}
