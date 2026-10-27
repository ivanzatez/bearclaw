// The phone's tab row, in the terminal dashboard's order (components/TabBar.tsx). Usage is on the
// Claw tab's card instead (and #/usage stays reachable); the brain is called Memory here. Videos is the
// phone's own (no dashboard tab): videos sessions made, to watch and ask about by voice.

export type TabId = 'sessions' | 'claw' | 'prs' | 'tasks' | 'todo' | 'memory' | 'notepad' | 'videos' | 'settings'

export const TABS: readonly { id: TabId; label: string; route: string }[] = [
  { id: 'sessions', label: 'Sessions', route: '/' },
  { id: 'claw', label: 'Claw', route: '/claw' },
  { id: 'prs', label: 'PRs', route: '/prs' },
  { id: 'tasks', label: 'Tasks', route: '/tasks' },
  { id: 'todo', label: 'Todo', route: '/todo' },
  { id: 'memory', label: 'Memory', route: '/memory' },
  { id: 'notepad', label: 'Notepad', route: '/notepad' },
  { id: 'videos', label: 'Videos', route: '/videos' },
  { id: 'settings', label: 'Settings', route: '/settings' },
]

/** The tab a hash route belongs to (`/memory/x.md` → memory), or null off the tab row (a session, jobs). */
export function activeTab(route: string): TabId | null {
  if (route === '/' || route === '') return 'sessions'
  const first = route.split('/')[1]
  return TABS.find((t) => t.id !== 'sessions' && t.route === `/${first}`)?.id ?? null
}
