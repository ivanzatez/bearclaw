import { expect, test } from 'vitest'

import { TABS, activeTab } from './tabs'

test('the tab row follows the terminal dashboard', () => {
  expect(TABS.map((t) => t.label)).toEqual(['Sessions', 'Claw', 'PRs', 'Tasks', 'Todo', 'Memory', 'Notepad', 'QA', 'Settings'])
})

test('activeTab maps a hash route to its tab', () => {
  expect(activeTab('/')).toBe('sessions')
  expect(activeTab('/claw')).toBe('claw')
  expect(activeTab('/prs/open')).toBe('prs')
  expect(activeTab('/memory/macbook/notes/x.md')).toBe('memory')
  expect(activeTab('/usage')).toBeNull()
  expect(activeTab('/s/mac/abc')).toBeNull()
  expect(activeTab('/jobs')).toBeNull()
  expect(activeTab('/qa/subagents-ab12')).toBe('qa')
  expect(activeTab('/videos/subagents')).toBeNull()
})
