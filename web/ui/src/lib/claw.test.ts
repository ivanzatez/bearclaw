import { expect, test } from 'vitest'

import type { BearclawResponse, PrsResponse, UsageResponse } from '@/api/types'

import { clawStats } from './claw'

const row = (id: string, status: string, extra = {}) => ({ host: 'mac', session_id: id, name: id, cwd: '/x', status, ...extra })
const bearclaw = {
  self: 'mac',
  hosts: [{ name: 'mac', ok: true, sessions: [row('a', 'busy'), row('b', 'idle'), row('c', 'waiting'), row('claw', 'busy', { claw: true })] }],
} as unknown as BearclawResponse

test('clawStats counts sessions without Claw, PRs waiting on you, and usage', () => {
  const prs = { prs: [{ column: 'waitingOnMe' }, { column: 'draftMine' }, { column: 'waitingOnMe' }] } as unknown as PrsResponse
  const usage = { limits: [{ group: 'session', percent: 34.4, active: true }, { group: 'weekly', percent: 61, active: true }] } as unknown as UsageResponse
  expect(clawStats({ bearclaw, prs, usage, todo: 4, brain: 15 })).toEqual({
    sessions: 3,
    busy: 1,
    idle: 1,
    waiting: 1,
    prWaiting: 2,
    todo: 4,
    usage: '34% session · 61% week',
    brain: 15,
  })
})

test('clawStats leaves what is not loaded unknown', () => {
  const s = clawStats({ bearclaw: null, prs: null, usage: null, todo: null, brain: null })
  expect(s).toMatchObject({ sessions: 0, prWaiting: null, usage: null, todo: null, brain: null })
})
