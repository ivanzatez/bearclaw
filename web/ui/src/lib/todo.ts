// The Todo tab (screens/TodoScreen.tsx): items by state, newest change first. Pure.
import type { TodoItem } from '@/api/types'

export function todoGroups(items: TodoItem[]): Record<TodoItem['status'], TodoItem[]> {
  const out: Record<TodoItem['status'], TodoItem[]> = { doing: [], open: [], done: [] }
  for (const it of items) (out[it.status] ?? out.open).push(it)
  for (const k of Object.keys(out) as TodoItem['status'][]) out[k].sort((a, b) => b.updatedMs - a.updatedMs)
  return out
}
