import { expect, test } from 'vitest'

import type { Message } from '@/api/types'
import { beatsOf, continueMessage, layoutDiagram, nextContinue, parseBeat, parseSlide, questionMessage, workingOn } from './qa'

const reply = 'Glavni agent ostaje na jednom modelu. Subagent je tool.\n\n```slide\n{"title": "Subagent je tool", "points": ["delegate_task"], "code": {"path": "src/x.ts", "start": 120, "text": "a\\nb\\n", "highlight": [2]}}\n```'

test('parseBeat splits what is said from its slide', () => {
  const { speech, slide } = parseBeat(reply)
  expect(speech).toBe('Glavni agent ostaje na jednom modelu. Subagent je tool.')
  expect(slide?.title).toBe('Subagent je tool')
  expect(slide?.code).toEqual({ path: 'src/x.ts', start: 120, text: 'a\nb', highlight: [2] })
  expect(parseBeat('Just talk, **no** slide.')).toEqual({ speech: 'Just talk, no slide.', slide: null })
  // A reply cut off inside its slide: no slide, and none of its JSON is read aloud.
  expect(parseBeat('Said.\n```slide\n{"title": "Hal').speech).toBe('Said.')
})

test('parseSlide: a title is required; a diagram keeps only edges between its nodes', () => {
  expect(parseSlide('{"points": ["x"]}')).toBeNull()
  expect(parseSlide('nope')).toBeNull()
  const s = parseSlide(JSON.stringify({ title: 'T', end: true, diagram: { nodes: [{ id: 'a', label: 'A' }, { id: 'b' }], edges: [['a', 'b', 'calls'], ['a', 'zz']] } }))
  expect(s?.end).toBe(true)
  expect(s?.diagram?.nodes.map((n) => n.label)).toEqual(['A', 'b'])
  expect(s?.diagram?.edges).toEqual([['a', 'b', 'calls']])
})

const user = (text: string, queued?: boolean): Message => ({ role: 'user', kind: 'user', text, queued })
const said = (text: string, final?: boolean): Message => ({ role: 'assistant', kind: 'assistant', text, final })

test('beatsOf pairs every message the screen sent with its reply, and knows when it is settled', () => {
  const list = [
    user('[qa · c-0] # QA session…'),
    said('Reading the brief.', false),
    said('Beat one.'),
    user(continueMessage(1)),
    said('Beat two.'),
    user(questionMessage('q-ab', 'zašto tool?', 'Subagent je tool')),
    said('Let me look at the code.', false),
  ]
  const busy = beatsOf(list, 'busy')
  expect(busy.map((b) => [b.tag, b.kind, b.reply, b.done])).toEqual([
    ['c-0', 'talk', 'Beat one.', true],
    ['c-1', 'talk', 'Beat two.', true],
    ['q-ab', 'answer', null, false],
  ])
  expect(busy[2].question).toBe('zašto tool?')
  const done = beatsOf([...list, said('Zato što cache ostaje topao.')], 'idle')
  expect(done[2]).toMatchObject({ reply: 'Zato što cache ostaje topao.', done: true })
  // Interrupted before it replied: settled, nothing to say.
  const cut = beatsOf([user(continueMessage(2)), said('Starting…', false), user('[Request interrupted by user]')], 'idle')
  expect(cut[0]).toMatchObject({ reply: null, done: true })
  expect(nextContinue(done)).toBe(2)
  expect(workingOn(list)).toBe('Let me look at the code.')
  expect(workingOn([...list, user('x')])).toBeNull()
})

test('layoutDiagram puts a node one column after whatever points at it', () => {
  const d = { nodes: ['user', 'main', 'fast', 'strong'].map((id) => ({ id, label: id, note: null })), edges: [['user', 'main', null], ['main', 'fast', null], ['main', 'strong', null]] as [string, string, null][] }
  const l = layoutDiagram(d, { colW: 100, rowH: 50 })
  const at = (id: string) => l.pos.get(id)!
  expect([at('user').x, at('main').x, at('fast').x, at('strong').x]).toEqual([0, 100, 200, 200])
  expect(at('fast').y).not.toBe(at('strong').y)
  expect([l.width, l.height]).toEqual([300, 100])
  // A cycle settles instead of looping.
  const c = layoutDiagram({ nodes: [{ id: 'a', label: 'a', note: null }, { id: 'b', label: 'b', note: null }], edges: [['a', 'b', null], ['b', 'a', null]] })
  expect(c.nodes).toHaveLength(2)
})
