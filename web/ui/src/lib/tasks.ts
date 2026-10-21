// The Tasks tab (screens/TasksScreen.tsx): Productive tasks grouped by status or project, filtered,
// with how close each is to its due date. Pure.
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

/** Every word must match the title, project, status or `#number`. */
export function filterTasks(tasks: ProductiveTask[], q: string): ProductiveTask[] {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return tasks
  return tasks.filter((t) => {
    const hay = `${t.title} ${t.project} ${t.status} #${t.number}`.toLowerCase()
    return words.every((w) => hay.includes(w))
  })
}

export type DueTone = 'overdue' | 'soon' | null

/** `overdue` past its due date, `soon` within 3 days (`today` is `YYYY-MM-DD`, local). */
export function dueTone(dueDate: string | null, today: string): DueTone {
  if (!dueDate) return null
  if (dueDate < today) return 'overdue'
  const days = (Date.parse(dueDate) - Date.parse(today)) / 86_400_000
  return days <= 3 ? 'soon' : null
}

/** Today as `YYYY-MM-DD` in local time. */
export function localDay(now = new Date()): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`
}

/** What a session started on a task is told first. */
export function taskPromptFor(t: Pick<ProductiveTask, 'number' | 'title' | 'url'>, prompt: string): string {
  const head = `Productive task #${t.number} "${t.title}": ${t.url}.`
  return prompt.trim() ? `${head} ${prompt.trim()}` : head
}
