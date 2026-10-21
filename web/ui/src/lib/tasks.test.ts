import { expect, test } from 'vitest'

import type { ProductiveTask } from '@/api/types'

import { groupTasks } from './tasks'

const task = (id: string, status: string, project: string): ProductiveTask => ({ id, number: id, title: `t${id}`, project, status, dueDate: null, updatedAt: '', url: '', description: '', statusCategory: 1 })

test('groupTasks keeps the board order by status, and sorts projects by name', () => {
  const tasks = [task('1', 'In progress', 'Web'), task('2', 'To do', 'API'), task('3', 'In progress', 'API')]
  expect(groupTasks(tasks, 'status').map((g) => [g.label, g.tasks.map((t) => t.id)])).toEqual([
    ['In progress', ['1', '3']],
    ['To do', ['2']],
  ])
  expect(groupTasks(tasks, 'project').map((g) => g.label)).toEqual(['API', 'Web'])
})
