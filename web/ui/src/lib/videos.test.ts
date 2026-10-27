import { expect, test } from 'vitest'

import type { Message, VideoScene, VideoSlide } from '@/api/types'
import { answerTo, askMessage, fmtTime, lineAt, sceneAt, slideSince } from './videos'

const scene = (id: string, start: number, dur: number, lines: [number, number, string][] = []): VideoScene => ({
  id,
  name: id,
  start,
  dur,
  visual: dur,
  anchors: null,
  lines: lines.map(([t0, t1, text]) => ({ t0, t1, text })),
})
const scenes = [scene('open', 0, 10, [[0.5, 3, 'Hello.'], [3.5, 6, 'Second.']]), scene('call', 10, 12, [[10.5, 14, 'Third.']])]

test('fmtTime reads minutes and seconds', () => {
  expect(fmtTime(0)).toBe('0:00')
  expect(fmtTime(83.9)).toBe('1:23')
  expect(fmtTime(-2)).toBe('0:00')
})

test('sceneAt and lineAt follow the narration clock', () => {
  expect(sceneAt(scenes, 0)).toBe(0)
  expect(sceneAt(scenes, 9.99)).toBe(0)
  expect(sceneAt(scenes, 10)).toBe(1)
  expect(sceneAt(scenes, 500)).toBe(1)
  expect(lineAt(scenes[0], 0.2)).toBeNull()
  expect(lineAt(scenes[0], 1)?.text).toBe('Hello.')
  expect(lineAt(scenes[0], 3.2)?.text).toBe('Hello.')
  expect(lineAt(scenes[0], 4)?.text).toBe('Second.')
})

test('askMessage tells the session where the viewer paused, how to answer, and where slides go', () => {
  const m = askMessage({ title: 'Subagents', scene: scenes[1], line: scenes[1].lines[0], t: 12.3, question: ' why a tool? ', slidesDir: '/v/slides', tag: 'q-abc' })
  expect(m.startsWith('[bearclaw video · q-abc]')).toBe(true)
  expect(m).toContain('paused at 0:12, in the scene "call". The last line I heard: "Third."')
  expect(m).toContain('My question (spoken and transcribed, so expect misheard words): why a tool?')
  expect(m).toContain('/v/slides/<short-name>.json')
})

const msg = (kind: Message['kind'], text: string, final?: boolean): Message => ({ role: kind === 'user' ? 'user' : 'assistant', kind, text, final })

test('answerTo: the last final reply after the tagged question, once the session stops working', () => {
  const list = [
    msg('assistant', 'old answer'),
    msg('user', '[bearclaw video · q-1] first'),
    msg('assistant', 'answer one'),
    msg('user', '[bearclaw video · q-2] second'),
    msg('assistant', 'Let me look at the code.', false),
    msg('assistant', 'Answer two.'),
  ]
  expect(answerTo(list, 'q-2', 'idle')).toBe('Answer two.')
  expect(answerTo(list, 'q-2', 'busy')).toBeNull()
  expect(answerTo(list, 'q-1', 'idle')).toBe('Answer two.')
  expect(answerTo(list.slice(0, 5), 'q-2', 'idle')).toBeNull()
  expect(answerTo(list, 'q-9', 'idle')).toBeNull()
})

test('slideSince picks the newest slide written after the question', () => {
  const s = (name: string, mtime: number): VideoSlide => ({ name, mtime, title: name, points: [], code: null })
  expect(slideSince([s('a', 1), s('b', 5), s('c', 9)], 4)?.name).toBe('c')
  expect(slideSince([s('a', 1)], 4)).toBeNull()
})
