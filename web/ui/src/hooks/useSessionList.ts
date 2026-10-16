import { useMemo, useState } from 'react'

import { useBearclaw } from '@/hooks/useBearclaw'
import { usePersistentState } from '@/hooks/usePersistentState'
import { relTime } from '@/lib/format'
import { allSessions, findStatusFilter, listView, statusSummary, type StatusFilterId } from '@/lib/sessions'

export const ALL_HOSTS = '*'

/**
 * The session list's state, shared by the mobile list screen and the desktop sidebar:
 * search, status + host filters (persisted in `bearclaw.filter`; "Scheduled" is a status filter),
 * the filtered view, per-host counts, the header summary and the "updated Xs ago" note.
 */
export function useSessionList(now: number) {
  const { bearclaw, bearclawAt, error, refreshing } = useBearclaw()
  const [query, setQuery] = useState('')
  const [status, setStatus] = usePersistentState<StatusFilterId>('bearclaw.filter', 'all', (raw) => findStatusFilter(raw).id)
  const [hostFilter, setHostFilter] = usePersistentState<string>('bearclaw.hostFilter', ALL_HOSTS)

  const hosts = bearclaw?.hosts ?? []
  const hostNames = hosts.map((h) => h.name)
  // A remembered host that is no longer configured means "all".
  const host = hostFilter !== ALL_HOSTS && hostNames.includes(hostFilter) ? hostFilter : null

  const view = useMemo(() => listView(bearclaw, { query, host, status }), [bearclaw, query, host, status])
  const hostCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const s of allSessions(bearclaw)) counts.set(s.host, (counts.get(s.host) ?? 0) + 1)
    return counts
  }, [bearclaw])
  const summary = useMemo(() => statusSummary(bearclaw), [bearclaw])

  const unreachable = hosts.filter((h) => h.ok === false && (host == null || h.name === host))

  let note: { text: string; error?: boolean }
  if (!bearclaw && refreshing) note = { text: 'loading…' }
  else if (error) note = { text: 'offline — retrying', error: true }
  else if (refreshing) note = { text: 'refreshing…' }
  else note = { text: bearclawAt ? `updated ${relTime(bearclawAt, now)} ago` : '' }

  return {
    bearclaw,
    error,
    query,
    setQuery,
    status,
    setStatus,
    setHostFilter,
    hosts,
    hostNames,
    host,
    view,
    hostCounts,
    summary,
    unreachable,
    note,
  }
}

export type SessionListState = ReturnType<typeof useSessionList>
