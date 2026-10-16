import { useCallback, useMemo, useState, type ReactNode } from 'react'

import { api, isAbortError } from '@/api/client'
import type { BearclawResponse } from '@/api/types'
import { BearclawContext, loadSnapshot, saveSnapshot, type BearclawState } from '@/hooks/useBearclaw'
import { usePoller } from '@/hooks/usePoller'

export const BEARCLAW_POLL_MS = 5000

/** Polls /api/bearclaw every 5s (paused while the page is hidden) for the whole app. */
export function BearclawProvider({ children, pollMs = BEARCLAW_POLL_MS }: { children: ReactNode; pollMs?: number }) {
  const [snap, setSnap] = useState(() => {
    const s = loadSnapshot()
    return { bearclaw: s?.bearclaw ?? null, at: s?.at ?? 0 }
  })
  const [error, setError] = useState<string | null>(null)
  const [refreshing, setRefreshing] = useState(false)

  const applyBearclaw = useCallback((data: BearclawResponse) => {
    const at = data?.snapshotAt || Date.now()
    setSnap({ bearclaw: data, at })
    saveSnapshot({ bearclaw: data, at })
  }, [])

  const poll = useCallback(
    async (signal: AbortSignal) => {
      setRefreshing(true)
      try {
        applyBearclaw(await api.bearclaw({ signal }))
        setError(null)
      } catch (err) {
        if (isAbortError(err)) throw err
        setError((err as Error)?.message || 'request failed')
      } finally {
        if (!signal.aborted) setRefreshing(false)
      }
    },
    [applyBearclaw],
  )

  const refresh = usePoller(poll, pollMs)

  const value = useMemo<BearclawState>(
    () => ({ bearclaw: snap.bearclaw, bearclawAt: snap.at, error, refreshing, refresh, applyBearclaw }),
    [snap, error, refreshing, refresh, applyBearclaw],
  )
  return <BearclawContext.Provider value={value}>{children}</BearclawContext.Provider>
}
