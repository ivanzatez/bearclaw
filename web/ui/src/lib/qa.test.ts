import { expect, test } from 'vitest'

import { goOnByItself, layoutDiagram, nextOrdered, parseSlide, statusSince, toSlide, type QaEvent } from './qa'

test('a slide: a title is required; code is trimmed; a diagram keeps only edges between its nodes', () => {
  expect(parseSlide('{"points": ["x"]}')).toBeNull()
  expect(parseSlide('nope')).toBeNull()
  expect(toSlide(null)).toBeNull()
  const s = toSlide({
    title: 'Subagent je tool',
    end: true,
    code: { path: 'src/x.ts', start: 120, text: 'a\nb\n', highlight: [2] },
    diagram: { nodes: [{ id: 'a', label: 'A' }, { id: 'b' }], edges: [['a', 'b', 'calls'], ['a', 'zz']] },
  })
  expect(s?.code).toEqual({ path: 'src/x.ts', start: 120, text: 'a\nb', highlight: [2] })
  expect(s?.end).toBe(true)
  expect(s?.diagram?.nodes.map((n) => n.label)).toEqual(['A', 'b'])
  expect(s?.diagram?.edges).toEqual([['a', 'b', 'calls']])
})

const ev = (seq: number, type: QaEvent['type'], text?: string): QaEvent => ({ seq, at: seq, type, text, sid: type === 'say' ? String(seq) : undefined })

test('the stream plays in order: sentences, slides and listening; status and questions show at once', () => {
  const list = [ev(1, 'slide'), ev(2, 'status', 'Reading x.ts'), ev(3, 'say', 'Hi.'), ev(4, 'listening')]
  expect(nextOrdered(list, 0)?.seq).toBe(1)
  expect(nextOrdered(list, 1)?.seq).toBe(3)
  expect(nextOrdered(list, 4)).toBeNull()
  expect(statusSince(list, 1)).toBe('Reading x.ts')
  expect(statusSince(list, 2)).toBeNull()
})

test('it goes on by itself only after answering a question, not at the end of the talk', () => {
  const end = [ev(1, 'say', 'Ask anything.'), ev(2, 'listening')]
  expect(goOnByItself(end, 2)).toBe(false)
  const answered = [...end, ev(3, 'interrupt'), ev(4, 'question', 'why?'), ev(5, 'say', 'Because.'), ev(6, 'listening')]
  expect(goOnByItself(answered, 6)).toBe(true)
  expect(goOnByItself([...answered, ev(7, 'listening')], 7)).toBe(false)
})

test('layoutDiagram puts a node one column after whatever points at it', () => {
  const d = { nodes: ['user', 'main', 'fast', 'strong'].map((id) => ({ id, label: id, note: null })), edges: [['user', 'main', null], ['main', 'fast', null], ['main', 'strong', null]] as [string, string, null][] }
  const l = layoutDiagram(d, { colW: 100, rowH: 50 })
  const at = (id: string) => l.pos.get(id)!
  expect([at('user').x, at('main').x, at('fast').x, at('strong').x]).toEqual([0, 100, 200, 200])
  expect(at('fast').y).not.toBe(at('strong').y)
  expect([l.width, l.height]).toEqual([300, 100])
  const c = layoutDiagram({ nodes: [{ id: 'a', label: 'a', note: null }, { id: 'b', label: 'b', note: null }], edges: [['a', 'b', null], ['b', 'a', null]] })
  expect(c.nodes).toHaveLength(2)
})
