import { expect, test } from 'vitest'

import type { ProductiveTask } from '@/api/types'

import { dueTone, filterTasks, groupTasks, localDay, taskPromptFor } from './tasks'

const task = (id: string, status: string, project: string, extra: Partial<ProductiveTask> = {}): ProductiveTask => ({ id, number: id, title: `t${id}`, project, status, dueDate: null, updatedAt: '', url: `https://p/${id}`, description: '', statusCategory: 1, ...extra })

test('groupTasks keeps the board order by status, and sorts projects by name', () => {
  const tasks = [task('1', 'In progress', 'Web'), task('2', 'To do', 'API'), task('3', 'In progress', 'API')]
  expect(groupTasks(tasks, 'status').map((g) => [g.label, g.tasks.map((t) => t.id)])).toEqual([
    ['In progress', ['1', '3']],
    ['To do', ['2']],
  ])
  expect(groupTasks(tasks, 'project').map((g) => g.label)).toEqual(['API', 'Web'])
})

test('filterTasks needs every word somewhere in the task', () => {
  const tasks = [task('11546', 'Triage', 'Product: AI', { title: 'MCP token returns 500' }), task('2', 'To do', 'Web', { title: 'Login page' })]
  expect(filterTasks(tasks, 'mcp ai').map((t) => t.id)).toEqual(['11546'])
  expect(filterTasks(tasks, '#11546').map((t) => t.id)).toEqual(['11546'])
  expect(filterTasks(tasks, '  ')).toHaveLength(2)
  expect(filterTasks(tasks, 'nope')).toEqual([])
})

test('dueTone: overdue, due within 3 days, or nothing', () => {
  expect(dueTone(null, '2026-10-21')).toBeNull()
  expect(dueTone('2026-10-20', '2026-10-21')).toBe('overdue')
  expect(dueTone('2026-10-21', '2026-10-21')).toBe('soon')
  expect(dueTone('2026-10-24', '2026-10-21')).toBe('soon')
  expect(dueTone('2026-10-25', '2026-10-21')).toBeNull()
  expect(localDay(new Date(2026, 0, 5))).toBe('2026-01-05')
})

test('taskPromptFor names the task, then what to do', () => {
  expect(taskPromptFor({ number: '7', title: 'Fix it', url: 'https://p/7' }, '')).toBe('Productive task #7 "Fix it": https://p/7.')
  expect(taskPromptFor({ number: '7', title: 'Fix it', url: 'https://p/7' }, ' Start with the spec. ')).toBe('Productive task #7 "Fix it": https://p/7. Start with the spec.')
})
