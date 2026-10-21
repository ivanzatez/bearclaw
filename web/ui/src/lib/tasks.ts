// The Tasks tab (screens/TasksScreen.tsx): Productive tasks grouped by status or project. Pure.
import type { ProductiveTask } from '@/api/types'

export type TaskGrouping = 'status' | 'project'

/**
 * Groups in first-seen order: the CLI already sorts tasks the way the board shows them
 * (status category, then position), so by status the board's order is kept.
 */
export function groupTasks(tasks: ProductiveTask[], by: TaskGrouping): { label: string; tasks: ProductiveTask[] }[] {
  const out = new Map<string, ProductiveTask[]>()
  for (const t of tasks) {
    const key = (by === 'status' ? t.status : t.project) || '—'
    out.set(key, [...(out.get(key) ?? []), t])
  }
  const groups = [...out].map(([label, list]) => ({ label, tasks: list }))
  return by === 'project' ? groups.sort((a, b) => a.label.localeCompare(b.label)) : groups
}
