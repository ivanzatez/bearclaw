import { createContext, useContext } from 'react'

import type { BearclawResponse } from '@/api/types'
import { storage } from '@/lib/storage'

export interface BearclawState {
  /** Last good /api/bearclaw body (or the localStorage snapshot before the first poll). */
  bearclaw: BearclawResponse | null
  /** When `bearclaw` was built (server `snapshotAt`, else receive time); 0 = never. */
  bearclawAt: number
  /** Last poll error message; null after a successful poll. */
  error: string | null
  /** A poll is in flight. */
  refreshing: boolean
  /** Poll now (e.g. after a spawn/kill). */
  refresh: () => void
  /** Record a fresh /api/bearclaw body fetched elsewhere (e.g. a spawn watcher). */
  applyBearclaw: (data: BearclawResponse) => void
}

export const BearclawContext = createContext<BearclawState | null>(null)

export function useBearclaw(): BearclawState {
  const ctx = useContext(BearclawContext)
  if (!ctx) throw new Error('useBearclaw() outside <BearclawProvider>')
  return ctx
}

// A reload paints the last list instantly while the first poll runs.
export const SNAPSHOT_KEY = 'bearclaw.snapshot'
export const SNAPSHOT_MAX_AGE_MS = 6 * 60 * 60 * 1000

export interface BearclawSnapshot {
  bearclaw: BearclawResponse
  at: number
}

export function loadSnapshot(now = Date.now()): BearclawSnapshot | null {
  const parsed = storage.getJSON<BearclawSnapshot>(SNAPSHOT_KEY)
  if (!parsed || !Array.isArray(parsed.bearclaw?.hosts) || !parsed.at) return null
  if (now - parsed.at > SNAPSHOT_MAX_AGE_MS) return null
  return parsed
}

export function saveSnapshot(snap: BearclawSnapshot): void {
  storage.setJSON(SNAPSHOT_KEY, snap)
}
