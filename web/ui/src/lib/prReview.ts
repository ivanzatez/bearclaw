// A code review session started from a PR card, as the terminal dashboard's Shift+R does: in the
// first start directory, named `review-<repo>-<n>`, told the PR and the review prompt. Pure.
import type { PullRequest, Session } from '@/api/types'

/** `review-<repo>-<n>`, as a tmux session name (letters, digits, `_` and `-`). */
export function reviewName(p: Pick<PullRequest, 'repo' | 'number'>): string {
  return `review-${p.repo}-${p.number}`.replace(/[^A-Za-z0-9_-]+/g, '-')
}

/** What the session is told first: the PR's line, then the review prompt (config `prs.reviewPrompt`). */
export function reviewPromptFor(p: Pick<PullRequest, 'owner' | 'repo' | 'number' | 'title' | 'url'>, prompt: string): string {
  return `Pull request ${p.owner}/${p.repo}#${p.number} "${p.title}": ${p.url}. ${prompt}`
}

const isReviewOf = (base: string, name: string | null | undefined) => !!name && (name === base || new RegExp(`^${base}-\\d+$`).test(name))

/** The live sessions reviewing this PR (by their tmux or Claude name). */
export function reviewSessions(p: Pick<PullRequest, 'repo' | 'number'>, sessions: Session[]): Session[] {
  const base = reviewName(p)
  return sessions.filter((s) => isReviewOf(base, s.tmux_session) || isReviewOf(base, s.name))
}

/** `base`, or `base-2`, `base-3`… when that tmux name is taken. */
export function nextReviewName(base: string, taken: Set<string>): string {
  if (!taken.has(base)) return base
  for (let i = 2; ; i++) if (!taken.has(`${base}-${i}`)) return `${base}-${i}`
}
