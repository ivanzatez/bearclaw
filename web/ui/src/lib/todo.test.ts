import { expect, test } from 'vitest'

import type { TodoItem } from '@/api/types'

import { todoGroups } from './todo'

const item = (id: number, status: TodoItem['status'], updatedMs: number): TodoItem => ({ id, text: `t${id}`, status, session: null, sessionLabel: null, taskUrl: null, createdMs: 0, updatedMs })

test('todoGroups splits by state, the latest change first', () => {
  const g = todoGroups([item(1, 'done', 5), item(2, 'doing', 1), item(3, 'open', 2), item(4, 'doing', 9)])
  expect(g.doing.map((i) => i.id)).toEqual([4, 2])
  expect(g.open.map((i) => i.id)).toEqual([3])
  expect(g.done.map((i) => i.id)).toEqual([1])
})
