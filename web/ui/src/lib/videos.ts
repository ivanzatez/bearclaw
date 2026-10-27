// The Videos screen (#/videos/<id>): pure helpers. A video is narrated scenes a session made
// (web/lib/videos.mjs); you pause it and ask by voice, the question goes to that session with
// where you paused, its spoken answer comes back and can bring an answer slide with it.
import type { Message, VideoLine, VideoScene, VideoSlide } from '@/api/types'

/** 83.4 → "1:23". */
export function fmtTime(s: number): string {
  const v = Math.max(0, Math.floor(s))
  return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, '0')}`
}

/** The index of the scene playing at `t` (the last one that has started). */
export function sceneAt(scenes: VideoScene[], t: number): number {
  let i = 0
  scenes.forEach((s, k) => {
    if (t >= s.start) i = k
  })
  return i
}

/** The line being said at `t`, or the last one said in this scene (null before its first). */
export function lineAt(scene: VideoScene | undefined, t: number): VideoLine | null {
  let cur: VideoLine | null = null
  for (const l of scene?.lines ?? []) if (t >= l.t0 - 0.05) cur = l
  return cur
}

/** What every question message carries, so its answer can be found among the session's messages. */
export const ASK_MARK = '[bearclaw video'

/** The message the session gets for a question asked at `t` (`tag` names this question). */
export function askMessage({ title, scene, line, t, question, slidesDir, tag }: {
  title: string
  scene: VideoScene | undefined
  line: VideoLine | null
  t: number
  question: string
  slidesDir: string
  tag: string
}): string {
  const where = `paused at ${fmtTime(t)}${scene ? `, in the scene "${scene.name}"` : ''}${line ? `. The last line I heard: "${line.text}"` : ''}`
  return [
    `${ASK_MARK} · ${tag}] I'm watching your video "${title}" and ${where}.`,
    `My question (spoken and transcribed, so expect misheard words): ${question.trim()}`,
    'Answer for the ear: your reply is read aloud to me. Two to five short spoken sentences, no markdown, no lists, no code blocks, no paths. Answer in the language of my question. Look at the code first if the question needs it.',
    `If a picture would help, first write one answer slide as JSON to ${slidesDir}/<short-name>.json: {"title": "...", "points": ["...", "..."], "code": "optional short snippet"}. It is shown on the video while your answer is read: a title, up to 5 short points, at most 15 lines of code.`,
  ].join('\n\n')
}

/**
 * The session's answer to the question tagged `tag`: its last final reply after that question,
 * once it has stopped working (null while it is still at it, or has not seen the question yet).
 */
export function answerTo(messages: Message[], tag: string, status: string | null | undefined): string | null {
  if (status === 'busy') return null
  const asked = messages.findLastIndex((m) => m.kind === 'user' && m.text?.includes(`${ASK_MARK} · ${tag}]`))
  if (asked < 0) return null
  for (let i = messages.length - 1; i > asked; i--) {
    const m = messages[i]
    if (m.kind === 'assistant' && m.final !== false && m.text?.trim()) return m.text.trim()
  }
  return null
}

/** The newest slide written since `since` (ms), if any. */
export function slideSince(slides: VideoSlide[], since: number): VideoSlide | null {
  const fresh = slides.filter((s) => s.mtime >= since)
  return fresh.length ? fresh[fresh.length - 1] : null
}

/** A short random tag for one question. */
export const questionTag = () => `q-${Math.random().toString(36).slice(2, 8)}`
