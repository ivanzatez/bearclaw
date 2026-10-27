// QA sessions (#/qa/<id>): pure helpers. A presenter session talks you through someone's work
// a beat at a time (web/lib/qa.mjs writes its brief): each reply is a few spoken sentences and
// one slide in a ```slide block. The screen sends "[qa · c-N] continue" for the next beat and
// "[qa · q-…] question …" when you interrupt; these helpers read the beats back out of its
// transcript.
import type { Message } from '@/api/types'

export const QA_MARK = '[qa · '

export interface QaCode {
  path: string | null
  start: number
  text: string
  highlight: number[]
}

export interface QaNode {
  id: string
  label: string
  note: string | null
}

export interface QaSlide {
  title: string
  points: string[]
  code: QaCode | null
  diagram: { nodes: QaNode[]; edges: [string, string, string | null][] } | null
  end: boolean
}

export interface Beat {
  /** `c-3` (the talk's beat 3) or `q-ab12cd` (a question). */
  tag: string
  /** Order among the beats asked for. */
  seq: number
  kind: 'talk' | 'answer'
  question: string | null
  /** The presenter's reply; null while it works on it (or it was interrupted before replying). */
  reply: string | null
  /** Settled: replied and done, or passed (a later message came first). */
  done: boolean
}

const str = (v: unknown, max: number) => (typeof v === 'string' ? v.slice(0, max) : '')

/** A slide's JSON → a slide, leniently; null when it has no title. */
export function parseSlide(raw: string): QaSlide | null {
  let s: Record<string, unknown>
  try {
    s = JSON.parse(raw)
  } catch {
    return null
  }
  if (!s || typeof s !== 'object' || !str(s.title, 200).trim()) return null
  const code = s.code && typeof s.code === 'object' ? (s.code as Record<string, unknown>) : null
  const diagram = s.diagram && typeof s.diagram === 'object' ? (s.diagram as Record<string, unknown>) : null
  const nodes: QaNode[] = Array.isArray(diagram?.nodes)
    ? (diagram.nodes as Record<string, unknown>[])
        .filter((n) => n && (typeof n.id === 'string' || typeof n.label === 'string'))
        .slice(0, 12)
        .map((n) => ({ id: str(n.id, 60) || str(n.label, 60), label: str(n.label, 60) || str(n.id, 60), note: str(n.note, 80) || null }))
    : []
  const ids = new Set(nodes.map((n) => n.id))
  const edges = Array.isArray(diagram?.edges)
    ? (diagram.edges as unknown[])
        .filter((e): e is unknown[] => Array.isArray(e) && ids.has(String(e[0])) && ids.has(String(e[1])))
        .slice(0, 20)
        .map((e) => [String(e[0]), String(e[1]), typeof e[2] === 'string' ? e[2].slice(0, 40) : null] as [string, string, string | null])
    : []
  return {
    title: str(s.title, 200).trim(),
    points: Array.isArray(s.points) ? s.points.map((p) => String(p).slice(0, 240)).slice(0, 6) : [],
    code:
      code && typeof code.text === 'string' && code.text.trim()
        ? {
            path: str(code.path, 200) || null,
            start: Number.isFinite(Number(code.start)) && Number(code.start) > 0 ? Math.floor(Number(code.start)) : 1,
            text: code.text.replace(/\s+$/, '').split('\n').slice(0, 30).join('\n'),
            highlight: Array.isArray(code.highlight) ? code.highlight.map(Number).filter((n) => Number.isInteger(n) && n > 0) : [],
          }
        : null,
    diagram: nodes.length ? { nodes, edges } : null,
    end: s.end === true,
  }
}

/** A reply → what is said (the slide and any code left out) and its slide. */
export function parseBeat(reply: string): { speech: string; slide: QaSlide | null } {
  const fence = /```slide\s*\n([\s\S]*?)(?:```|$)/g
  let slide: QaSlide | null = null
  for (const m of reply.matchAll(fence)) slide = parseSlide(m[1].trim()) ?? slide
  const speech = reply
    .replace(fence, ' ')
    .replace(/```[\s\S]*?(```|$)/g, ' ')
    .replace(/[*_`#>]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
  return { speech, slide }
}

/** The beats in the presenter's transcript: every message the screen sent and the reply to it. */
export function beatsOf(messages: Message[], status: string | null | undefined): Beat[] {
  const out: Beat[] = []
  messages.forEach((m, i) => {
    if (m.kind !== 'user' || !m.text?.startsWith(QA_MARK)) return
    const tag = m.text.slice(QA_MARK.length).split(']')[0].trim()
    let reply: string | null = null
    let passed = false
    for (let j = i + 1; j < messages.length; j++) {
      const n = messages[j]
      if (n.kind === 'user' || n.role === 'user') {
        passed = true
        break
      }
      if (n.kind === 'assistant' && n.final !== false && n.text?.trim()) reply = n.text
    }
    const question = tag.startsWith('q-') ? m.text.replace(/^\[qa · [^\]]*\] question \(.*?\): /s, '').trim() : null
    out.push({
      tag,
      seq: out.length,
      kind: tag.startsWith('q-') ? 'answer' : 'talk',
      question,
      reply: passed || status !== 'busy' ? reply : null,
      done: passed || (status !== 'busy' && !m.queued),
    })
  })
  return out
}

/** The presenter's latest words while it works (narration between its tool calls), if any. */
export function workingOn(messages: Message[]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.kind === 'user') return null
    if (m.kind === 'assistant' && m.text?.trim()) return m.text.trim().split('\n')[0].slice(0, 160)
  }
  return null
}

export const continueMessage = (n: number) => `${QA_MARK}c-${n}] continue`

export function questionMessage(tag: string, question: string, slide: string | null): string {
  return `${QA_MARK}${tag}] question (they interrupted you${slide ? ` on the slide "${slide}"` : ''}): ${question.trim()}`
}

/** The next talk beat's number: one after the highest `c-N` asked for. */
export function nextContinue(beats: Beat[]): number {
  return beats.reduce((n, b) => (b.tag.startsWith('c-') ? Math.max(n, Number(b.tag.slice(2)) + 1 || n) : n), 1)
}

export const questionTag = () => `q-${Math.random().toString(36).slice(2, 8)}`

export interface Placed {
  id: string
  label: string
  note: string | null
  x: number
  y: number
}

/**
 * Boxes and arrows laid out left to right: a node's column is its longest path from a source,
 * nodes in a column stack in the order given, columns centred. Box size is the caller's.
 */
export function layoutDiagram(d: NonNullable<QaSlide['diagram']>, { colW = 260, rowH = 120 } = {}) {
  const layer = new Map(d.nodes.map((n) => [n.id, 0]))
  // Longest path, bounded so a cycle cannot loop forever.
  for (let pass = 0; pass < d.nodes.length; pass++) {
    let moved = false
    for (const [a, b] of d.edges) {
      if (a === b) continue
      const want = (layer.get(a) ?? 0) + 1
      if ((layer.get(b) ?? 0) < want && want < d.nodes.length) {
        layer.set(b, want)
        moved = true
      }
    }
    if (!moved) break
  }
  const cols: string[][] = []
  for (const n of d.nodes) (cols[layer.get(n.id) ?? 0] ??= []).push(n.id)
  const dense = cols.filter(Boolean)
  const rows = Math.max(1, ...dense.map((c) => c.length))
  const pos = new Map<string, { x: number; y: number }>()
  dense.forEach((c, ci) => c.forEach((id, ri) => pos.set(id, { x: ci * colW, y: (ri + (rows - c.length) / 2) * rowH })))
  const nodes: Placed[] = d.nodes.map((n) => ({ ...n, ...pos.get(n.id)! }))
  return { nodes, edges: d.edges, width: dense.length * colW, height: rows * rowH, pos }
}
