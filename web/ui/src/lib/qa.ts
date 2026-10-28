// QA sessions (#/qa/<id>): pure helpers. A presenter session talks you through someone's work
// live with its MCP tools — say, show, listen, status (web/lib/qa.mjs writes its brief); the
// screen plays the stream of what it says and shows (web/lib/qa-live.mjs), in order.

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

const str = (v: unknown, max: number) => (typeof v === 'string' ? v.slice(0, max) : '')

/** A slide's JSON → a slide, leniently; null when it has no title. */
export function parseSlide(raw: string): QaSlide | null {
  try {
    return toSlide(JSON.parse(raw))
  } catch {
    return null
  }
}

/** A slide as the presenter's `show` sent it → a slide, leniently; null when it has no title. */
export function toSlide(raw: unknown): QaSlide | null {
  const s = raw as Record<string, unknown>
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

/** What the presenter's live stream carries (web/lib/qa-live.mjs). */
export interface QaEvent {
  seq: number
  at: number
  type: 'say' | 'slide' | 'status' | 'listening' | 'interrupt' | 'question'
  sid?: string
  text?: string
  slide?: unknown
  /** listening: the presenter paused because nobody heard it, not because the talk ended. */
  away?: boolean
}

/** Played in order (a slide goes up after the sentences said before it); the rest shows at once. */
export const ORDERED = new Set<QaEvent['type']>(['say', 'slide', 'listening'])

/**
 * The next event to play after `cursor`, if any. Nothing from before the last interruption: what
 * was on its way when you cut in (still in flight to this page) is never played.
 */
export function nextOrdered(events: QaEvent[], cursor: number): QaEvent | null {
  const cut = events.findLast((e) => e.type === 'interrupt')?.seq ?? 0
  return events.find((e) => e.seq > cursor && e.seq > cut && ORDERED.has(e.type)) ?? null
}

/**
 * The presenter is listening at `seq`: go on by itself? Right after answering a question (it asked
 * whether to go on), or when it only paused because nobody was listening; at the end of the talk it
 * waits for the user.
 */
export function goOnByItself(events: QaEvent[], seq: number): boolean {
  if (events.find((e) => e.seq === seq)?.away) return true
  const before = events.filter((e) => e.seq < seq)
  const lastListen = before.findLast((e) => e.type === 'listening')?.seq ?? 0
  return before.some((e) => e.type === 'question' && e.seq > lastListen)
}

/** The presenter's latest "what I am doing" line since it last spoke, if any. */
export function statusSince(events: QaEvent[], cursor: number): string | null {
  const s = events.findLast((e) => e.type === 'status')
  return s && s.seq > cursor && s.text ? s.text : null
}

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

/** Where you left off: the slide up and the sentence said when you had heard up to `heard`. */
export function resumeAt(events: QaEvent[], heard: number): { slide: QaSlide | null; caption: string } {
  const before = events.filter((e) => e.seq <= heard)
  return { slide: toSlide(before.findLast((e) => e.type === 'slide')?.slide), caption: before.findLast((e) => e.type === 'say')?.text ?? '' }
}

/** A transcript that is only sound tags ("[clicking]", "(music)"): nothing was said. */
export const onlySounds = (text: string) => !text.replace(/\[[^\]]*\]|\([^)]*\)/g, '').trim()

