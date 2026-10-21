// The phone's tab row, in the terminal dashboard's order (components/TabBar.tsx).

export type TabId = 'sessions' | 'claw' | 'prs' | 'tasks' | 'todo' | 'usage' | 'brain' | 'notepad' | 'settings'

export const TABS: readonly { id: TabId; label: string; route: string }[] = [
  { id: 'sessions', label: 'Sessions', route: '/' },
  { id: 'claw', label: 'Claw', route: '/claw' },
  { id: 'prs', label: 'PRs', route: '/prs' },
  { id: 'tasks', label: 'Tasks', route: '/tasks' },
  { id: 'todo', label: 'Todo', route: '/todo' },
  { id: 'usage', label: 'Usage', route: '/usage' },
  { id: 'brain', label: 'Brain', route: '/brain' },
  { id: 'notepad', label: 'Notepad', route: '/notepad' },
  { id: 'settings', label: 'Settings', route: '/settings' },
]

/** The tab a hash route belongs to (`/brain/x.md` → brain), or null off the tab row (a session, jobs). */
export function activeTab(route: string): TabId | null {
  if (route === '/' || route === '') return 'sessions'
  const first = route.split('/')[1]
  return TABS.find((t) => t.id !== 'sessions' && t.route === `/${first}`)?.id ?? null
}
