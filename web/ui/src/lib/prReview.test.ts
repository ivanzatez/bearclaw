import { expect, test } from 'vitest'

import type { Session } from '@/api/types'

import { nextReviewName, reviewName, reviewPromptFor, reviewSessions } from './prReview'

const pr = { owner: 'productiveio', repo: 'ai-agent', number: 2504, title: 'feat: x', url: 'https://github.com/productiveio/ai-agent/pull/2504' }

test('a review session is named and briefed like the dashboard does it', () => {
  expect(reviewName(pr)).toBe('review-ai-agent-2504')
  expect(reviewName({ repo: 'my.repo', number: 1 })).toBe('review-my-repo-1')
  expect(reviewPromptFor(pr, 'Review this pull request.')).toBe('Pull request productiveio/ai-agent#2504 "feat: x": https://github.com/productiveio/ai-agent/pull/2504. Review this pull request.')
})

test('reviewSessions finds the sessions by tmux or Claude name, suffixed ones too', () => {
  const s = (name: string, tmux: string | null) => ({ host: 'mac', session_id: name, name, tmux_session: tmux }) as unknown as Session
  const found = reviewSessions(pr, [s('review-ai-agent-2504', 'review-ai-agent-2504'), s('x', 'review-ai-agent-2504-2'), s('review-ai-agent-25041', null), s('other', 'other')])
  expect(found.map((x) => x.name)).toEqual(['review-ai-agent-2504', 'x'])
})

test('nextReviewName skips taken names', () => {
  expect(nextReviewName('review-a-1', new Set())).toBe('review-a-1')
  expect(nextReviewName('review-a-1', new Set(['review-a-1', 'review-a-1-2']))).toBe('review-a-1-3')
})
