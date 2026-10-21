import { createContext, useContext, useEffect, useMemo, useState } from 'react'

import { ApiError, api, isAbortError } from '@/api/client'
import type { NoteFile, NotesBook, NotesSearch, NotesTree } from '@/api/types'
import { useBearclaw } from '@/hooks/useBearclaw'
import { usePoller } from '@/hooks/usePoller'
import { indexNotes } from '@/lib/notes'

const TREE_POLL_MS = 30_000
const SEARCH_DEBOUNCE_MS = 180

/**
 * Which notebook the notes screen and its parts show: the `web.notes` explorer (default), or the
 * dashboard's Brain / Notepad (App.tsx provides it per route).
 */
export const NotesBookContext = createContext<NotesBook>('notes')
export const useNotesBook = () => useContext(NotesBookContext)

/** Hosts whose server has this notebook, self first (from /api/bearclaw host entries). */
export function useNotesHosts(book: NotesBook = 'notes'): { name: string; label: string | null }[] {
  const { bearclaw } = useBearclaw()
  return useMemo(
    () =>
      (bearclaw?.hosts ?? [])
        .filter((h) => (book === 'notes' ? h.notes : h.books?.includes(book)))
        .map((h) => ({ name: h.name, label: book === 'notes' ? (h.notes?.name ?? null) : null })),
    [bearclaw, book],
  )
}

// Last tree per notebook + host, kept across remounts so switching screens repaints at once.
const trees = new Map<string, NotesTree>()

/** The host's file list: at once from the cache, then polled every 30s while shown. */
export function useNotesTree(host: string | null) {
  const book = useNotesBook()
  const cacheKey = host ? `${book}\0${host}` : ''
  const [state, setState] = useState<{ host: string | null; tree: NotesTree | null; error: string | null }>(() => ({
    host: cacheKey || null,
    tree: cacheKey ? (trees.get(cacheKey) ?? null) : null,
    error: null,
  }))
  if (state.host !== (cacheKey || null)) setState({ host: cacheKey || null, tree: cacheKey ? (trees.get(cacheKey) ?? null) : null, error: null })
  const refresh = usePoller(
    async (signal) => {
      if (!host) return
      try {
        const tree = await api.notesTree(host, { signal, book })
        trees.set(cacheKey, tree)
        setState((s) => (s.host === cacheKey ? { host: cacheKey, tree, error: null } : s))
      } catch (err) {
        if (isAbortError(err)) return
        const error = err instanceof ApiError && err.status === 501 ? `Notes are not configured on ${host}` : (err as Error)?.message || 'could not load notes'
        setState((s) => (s.host === cacheKey ? { ...s, error } : s))
      }
    },
    TREE_POLL_MS,
    { enabled: !!host },
  )
  const tree = state.host === (cacheKey || null) ? state.tree : null
  const index = useMemo(() => indexNotes(tree?.files ?? []), [tree])
  return { tree, index, error: state.host === (cacheKey || null) ? state.error : null, refresh }
}

/** Debounced search; the previous request is aborted when the query changes. */
export function useNotesSearch(host: string | null, q: string) {
  const book = useNotesBook()
  const query = q.trim()
  const [state, setState] = useState<{ key: string; data: NotesSearch | null; error: string | null; loading: boolean }>({ key: '', data: null, error: null, loading: false })
  const key = host && query ? `${book}\0${host}\0${query}` : ''
  useEffect(() => {
    if (!key || !host) return
    const ctl = new AbortController()
    const timer = setTimeout(() => {
      setState((s) => ({ ...s, key, loading: true, error: null }))
      api
        .notesSearch(host, query, 60, { signal: ctl.signal, book })
        .then((data) => setState({ key, data, error: null, loading: false }))
        .catch((err) => {
          if (!isAbortError(err)) setState({ key, data: null, error: (err as Error)?.message || 'search failed', loading: false })
        })
    }, SEARCH_DEBOUNCE_MS)
    return () => {
      clearTimeout(timer)
      ctl.abort()
    }
  }, [key, host, query, book])
  if (!key) return { data: null, error: null, loading: false, pending: false }
  // Results of the previous query stay up (dimmed by the caller) until the new ones land.
  return { data: state.data, error: state.key === key ? state.error : null, loading: state.loading || state.key !== key, pending: state.key !== key }
}

/** One note; reloaded when the tree reports a new mtime for it. */
export function useNoteFile(host: string | null, path: string | null, mtime: number | undefined) {
  const book = useNotesBook()
  const [state, setState] = useState<{ key: string; file: NoteFile | null; error: string | null }>({ key: '', file: null, error: null })
  const key = host && path ? `${book}\0${host}\0${path}` : ''
  useEffect(() => {
    if (!key || !host || !path) return
    const ctl = new AbortController()
    api
      .noteFile(host, path, { signal: ctl.signal, book })
      .then((file) => setState({ key, file, error: null }))
      .catch((err) => {
        if (!isAbortError(err)) setState({ key, file: null, error: (err as Error)?.message || 'could not load the note' })
      })
    return () => ctl.abort()
  }, [key, host, path, mtime, book])
  if (!key || state.key !== key) return { file: null, error: null, loading: !!key }
  return { file: state.file, error: state.error, loading: false }
}
