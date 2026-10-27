import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { ChevronLeftIcon, Loader2Icon, MicIcon, PauseIcon, PlayIcon, PresentationIcon, SendIcon, SquareIcon } from 'lucide-react'
import { toast } from 'sonner'
import { Link, useLocation, useRoute } from 'wouter'

import { ApiError, api } from '@/api/client'
import type { Message, QaRecord, Session } from '@/api/types'
import { QaSlide } from '@/components/QaSlide'
import { ScreenHeader } from '@/components/ScreenHeader'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { useBearclaw } from '@/hooks/useBearclaw'
import { useNow } from '@/hooks/useNow'
import { usePoller } from '@/hooks/usePoller'
import { useSettings } from '@/hooks/useSettings'
import { useVoice } from '@/hooks/useVoice'
import { relTime } from '@/lib/format'
import { type Beat, beatsOf, continueMessage, nextContinue, parseBeat, questionMessage, questionTag, workingOn } from '@/lib/qa'
import { allSessions, sessionHref } from '@/lib/sessions'
import { sessionTitle } from '@/lib/title'
import { cn } from '@/lib/utils'
import { formatElapsed } from '@/lib/voice'

/** `#/qa` (the QA sessions, and a new one) and `#/qa/<id>` (one, live). */
export function QaScreen({ layout = 'screen' }: { layout?: 'screen' | 'pane' }) {
  const [open, params] = useRoute('/qa/:id')
  return open ? <QaLive key={params.id} id={params.id} layout={layout} /> : <QaList layout={layout} />
}

const presenterOf = (sessions: Session[], qa: QaRecord | null) => (qa ? (sessions.find((s) => s.tmux_session === qa.presenter.tmux) ?? null) : null)
const shell = (layout: 'screen' | 'pane') => cn('flex flex-col bg-background', layout === 'screen' ? 'min-h-app' : 'min-h-0 flex-1 overflow-y-auto')

function QaList({ layout }: { layout: 'screen' | 'pane' }) {
  const { self } = useSettings()
  const { bearclaw } = useBearclaw()
  const [, navigate] = useLocation()
  const now = useNow(30_000)
  const [list, setList] = useState<QaRecord[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [source, setSource] = useState('')
  const [topic, setTopic] = useState('')
  const [lang, setLang] = useState<'hr' | 'en'>('hr')
  const [busy, setBusy] = useState(false)
  usePoller(async (signal) => {
    try {
      setList((await api.qaSessions({ signal })).sessions)
      setError(null)
    } catch (err) {
      if ((err as Error)?.name !== 'AbortError') setError(err instanceof ApiError && err.status === 404 ? 'This server predates QA sessions: update bearclaw and restart its web server.' : (err as Error)?.message || 'could not load QA sessions')
    }
  }, 15_000)
  const sessions = useMemo(() => allSessions(bearclaw), [bearclaw])
  const sources = sessions.filter((s) => !s.claw && !s.operator && !(s.tmux_session ?? '').startsWith('qa-'))

  const start = async (e: FormEvent) => {
    e.preventDefault()
    if (!self || busy) return
    const src = sources.find((s) => `${s.host}/${s.session_id}` === source) ?? null
    if (!src && !topic.trim()) return toast.error('Pick a session or write a topic')
    setBusy(true)
    try {
      const qa = await api.createQa({ topic, lang, source: src ? { host: src.host, session: src.session_id, title: sessionTitle(src), cwd: src.cwd ?? null } : null })
      const spawn = (dir?: string) => api.spawn(self, { name: qa.presenter.tmux, dir, prompt: qa.prompt ?? '' })
      try {
        await spawn(src?.cwd ?? undefined)
      } catch (err) {
        // Its folder is not a start directory: the first one.
        if (!(err instanceof ApiError && err.status === 400 && src?.cwd)) throw err
        await spawn()
      }
      navigate(`/qa/${qa.id}`)
    } catch (err) {
      toast.error('QA session not started', { description: (err as Error)?.message })
      setBusy(false)
    }
  }

  return (
    <div className={shell(layout)}>
      <ScreenHeader tabs={layout === 'screen'}>
        <h1 className="flex min-h-8 items-center text-xl font-bold tracking-tight">QA</h1>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-3xl space-y-4 px-4 pt-3 pb-[max(1.5rem,env(safe-area-inset-bottom))]">
        <form onSubmit={start} className="space-y-3 rounded-xl border bg-card p-3">
          <p className="text-sm text-muted-foreground">A new session presents the work to you by voice, slide by slide. Interrupt it any time and ask: it answers from the real code.</p>
          <select
            value={source}
            onChange={(e) => setSource(e.target.value)}
            aria-label="Whose work"
            className="h-9 w-full rounded-md border bg-background px-2 text-sm"
          >
            <option value="">No session: just the topic below</option>
            {sources.map((s) => (
              <option key={`${s.host}/${s.session_id}`} value={`${s.host}/${s.session_id}`}>
                {sessionTitle(s)}
              </option>
            ))}
          </select>
          <Textarea value={topic} onChange={(e) => setTopic(e.target.value)} rows={2} placeholder="What should it present? (optional with a session)" aria-label="Topic" />
          <div className="flex flex-wrap items-center gap-2">
            {(['hr', 'en'] as const).map((l) => (
              <button
                key={l}
                type="button"
                aria-pressed={lang === l}
                onClick={() => setLang(l)}
                className={cn('h-8 rounded-full border px-3 text-xs', lang === l ? 'border-primary/50 bg-primary/10 text-foreground' : 'text-muted-foreground')}
              >
                {l === 'hr' ? 'Hrvatski' : 'English'}
              </button>
            ))}
            <Button type="submit" size="sm" className="ml-auto" disabled={busy || !self}>
              {busy ? <Loader2Icon className="animate-spin" /> : <PresentationIcon />} Start QA
            </Button>
          </div>
        </form>
        {error ? <p className="text-sm text-status-waiting">{error}</p> : null}
        <div className="space-y-2">
          {(list ?? []).map((q) => {
            const live = presenterOf(sessions, q)
            return (
              <Link key={q.id} href={`/qa/${q.id}`} className="flex items-center gap-3 rounded-lg border bg-card p-3 text-sm hover:border-primary/40 hover:bg-muted/40">
                <PresentationIcon className="size-5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium">{q.title}</span>
                  <span className="block text-xs text-muted-foreground">
                    {live ? 'presenter running' : 'presenter closed'} · {q.lang === 'hr' ? 'Hrvatski' : 'English'} · {relTime(q.createdAt, now) || 'just now'}
                  </span>
                </span>
              </Link>
            )
          })}
        </div>
      </div>
    </div>
  )
}

// 50 ms of silence: played inside a tap, it lets this element play the answers that come later (iOS).
function silentWav(): string {
  const n = 400
  const b = new Uint8Array(44 + n)
  const v = new DataView(b.buffer)
  const ascii = (o: number, t: string) => [...t].forEach((c, i) => (b[o + i] = c.charCodeAt(0)))
  ascii(0, 'RIFF'); v.setUint32(4, 36 + n, true); ascii(8, 'WAVEfmt ')
  v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true)
  v.setUint32(24, 8000, true); v.setUint32(28, 8000, true); v.setUint16(32, 1, true); v.setUint16(34, 8, true)
  ascii(36, 'data'); v.setUint32(40, n, true); b.fill(128, 44)
  return `data:audio/wav;base64,${btoa(String.fromCharCode(...b))}`
}

const AUTO_CONTINUE_MS = 6000

function QaLive({ id, layout }: { id: string; layout: 'screen' | 'pane' }) {
  const { voice: canTranscribe } = useSettings()
  const { bearclaw } = useBearclaw()
  const [qa, setQa] = useState<QaRecord | null>(null)
  const [msgs, setMsgs] = useState<Message[]>([])
  const [status, setStatus] = useState<string | null>(null)
  const [started, setStarted] = useState(false)
  const [held, setHeld] = useState(false)
  // talk: ask for the next beat on its own; answer: a question is being answered, then wait.
  const [mode, setMode] = useState<'talk' | 'answer'>('talk')
  const [playing, setPlaying] = useState<Beat | null>(null)
  const [shown, setShown] = useState<{ tag: string; speech: string; slide: ReturnType<typeof parseBeat>['slide'] } | null>(null)
  const [afterAnswer, setAfterAnswer] = useState(false)
  const [ended, setEnded] = useState(false)
  const [typed, setTyped] = useState('')
  const [played, setPlayed] = useState<ReadonlySet<string>>(() => new Set())
  // Beats asked for before an interruption: never read out (the talk moved on).
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(() => new Set())
  const sending = useRef<string | null>(null)
  const audio = useRef<HTMLAudioElement | null>(null)
  const voiced = useRef(new Map<string, Promise<string>>())
  const playingRef = useRef<string | null>(null)

  usePoller(async (signal) => {
    try {
      setQa(await api.qa(id, { signal }))
    } catch (err) {
      if ((err as Error)?.name !== 'AbortError') toast.error('QA session not found', { description: (err as Error)?.message })
    }
  }, 60_000)
  const presenter = presenterOf(allSessions(bearclaw), qa)
  usePoller(
    async (signal) => {
      if (!presenter) return
      const r = await api.messages(presenter.host, presenter.session_id, 200, { signal })
      setMsgs(r.messages)
      setStatus(r.status)
    },
    1000,
    { enabled: Boolean(presenter) },
  )
  const beats = useMemo(() => beatsOf(msgs, status), [msgs, status])
  useEffect(() => {
    if (sending.current && beats.some((b) => b.tag === sending.current)) sending.current = null
  }, [beats])
  const queue = beats.filter((b) => b.done && b.reply && !played.has(b.tag) && !skipped.has(b.tag) && b.tag !== playing?.tag)
  const pending = beats.find((b) => !b.done) ?? null

  useEffect(() => {
    const a = new Audio()
    a.preload = 'auto'
    audio.current = a
    const cache = voiced.current
    return () => {
      a.pause()
      cache.forEach((p) => p.then((u) => URL.revokeObjectURL(u)).catch(() => {}))
    }
  }, [])

  const voice = useCallback((b: Beat, speech: string) => {
    let p = voiced.current.get(b.tag)
    if (!p) {
      p = api.speak(speech).then((blob) => URL.createObjectURL(blob))
      voiced.current.set(b.tag, p)
    }
    return p
  }, [])

  const finish = useCallback((b: Beat, endOfTalk: boolean) => {
    setPlayed((p) => new Set(p).add(b.tag))
    playingRef.current = null
    setPlaying(null)
    if (endOfTalk) setEnded(true)
    if (b.kind === 'answer') setAfterAnswer(true)
  }, [])

  // Play the next beat that is ready: its slide goes up as its voice starts.
  const next = queue[0]
  useEffect(() => {
    if (!started || held || playingRef.current || !next) return
    const a = audio.current
    const { speech, slide } = parseBeat(next.reply ?? '')
    playingRef.current = next.tag
    const endOfTalk = Boolean(slide?.end)
    const begin = () => {
      setPlaying(next)
      setShown((prev) => ({ tag: next.tag, speech, slide: slide ?? prev?.slide ?? null }))
    }
    if (!speech || !a) {
      void Promise.resolve().then(() => {
        begin()
        finish(next, endOfTalk)
      })
      return
    }
    voice(next, speech)
      .then((url) => {
        if (playingRef.current !== next.tag) return
        begin()
        a.onended = () => playingRef.current === next.tag && finish(next, endOfTalk)
        a.src = url
        return a.play()
      })
      .catch((err) => {
        if (playingRef.current !== next.tag) return
        toast.error('Could not read it aloud', { description: (err as Error)?.message })
        begin()
        finish(next, endOfTalk)
      })
  }, [started, held, playing, next, voice, finish])

  // Have the following beat's voice ready before this one ends.
  const after = queue.find((b) => b.tag !== next?.tag)
  useEffect(() => {
    const b = playing ? next : after
    if (b?.reply) void voice(b, parseBeat(b.reply).speech).catch(() => {})
  }, [playing, next, after, voice])

  // Ask for the next beat while this one plays (one beat ahead, never more).
  const last = beats[beats.length - 1]
  const lastEnds = last?.reply ? Boolean(parseBeat(last.reply).slide?.end) : false
  const micBusy = useRef(false)
  useEffect(() => {
    if (!started || held || ended || mode !== 'talk' || !presenter || sending.current || micBusy.current) return
    if (status === 'busy' || pending || queue.length > 0 || !last || lastEnds) return
    const n = nextContinue(beats)
    sending.current = `c-${n}`
    api.send(presenter.host, presenter.session_id, continueMessage(n)).catch((err) => {
      sending.current = null
      toast.error('The presenter did not get "continue"', { description: (err as Error)?.message })
    })
  }, [started, held, ended, mode, presenter, status, pending, queue.length, last, lastEnds, beats])

  // After an answer: back to the talk on its own, unless you ask again or hold it.
  const resume = useCallback(() => {
    setAfterAnswer(false)
    setMode('talk')
  }, [])
  useEffect(() => {
    if (!afterAnswer || held || ended) return
    const t = window.setTimeout(resume, AUTO_CONTINUE_MS)
    return () => window.clearTimeout(t)
  }, [afterAnswer, held, ended, resume])

  const unlock = () => {
    const a = audio.current
    if (a && !a.src) {
      a.src = silentWav()
      a.play().catch(() => {})
    }
  }

  /** Stop talking now: what is said and what was ready to be said next. */
  const interrupt = () => {
    audio.current?.pause()
    const gone = beats.map((b) => b.tag)
    if (playing) setPlayed((p) => new Set(p).add(playing.tag))
    playingRef.current = null
    setPlaying(null)
    setSkipped((s) => new Set([...s, ...gone]))
    setAfterAnswer(false)
    if (presenter && status === 'busy') api.keys(presenter.host, presenter.session_id, 'Escape').catch(() => {})
  }

  const ask = async (question: string) => {
    const q = question.trim()
    if (!q || !presenter) return
    const tag = questionTag()
    sending.current = tag
    setMode('answer')
    setEnded(false)
    try {
      await api.send(presenter.host, presenter.session_id, questionMessage(tag, q, shown?.slide?.title ?? null))
      setTyped('')
    } catch (err) {
      sending.current = null
      toast.error('Question not sent', { description: (err as Error)?.message })
    }
  }

  const mic = useVoice({ onText: (t) => void ask(t), hint: qa?.title ?? '' })
  useEffect(() => {
    micBusy.current = mic.state !== 'idle'
  }, [mic.state])
  const tick = useNow(500)
  const onMic = () => {
    if (mic.state === 'recording') return mic.stop()
    if (mic.state === 'transcribing') return mic.cancel()
    unlock()
    setStarted(true)
    interrupt()
    void mic.start()
  }
  const onType = (e: FormEvent) => {
    e.preventDefault()
    unlock()
    setStarted(true)
    interrupt()
    void ask(typed)
  }
  const onHold = () => {
    if (held) {
      setHeld(false)
      if (playing) void audio.current?.play().catch(() => {})
    } else {
      setHeld(true)
      audio.current?.pause()
    }
  }

  const line = (() => {
    if (!qa) return 'Loading…'
    if (!presenter) return 'Starting the presenter…'
    if (presenter.status === 'waiting') return `The presenter waits on you in its terminal${presenter.waiting_for ? `: ${presenter.waiting_for}` : ''}`
    if (mic.state === 'recording') return `Listening… ${formatElapsed(tick - (mic.startedAt ?? tick))} — tap to send`
    if (mic.state === 'transcribing') return 'Transcribing…'
    if (!started) return beats.some((b) => b.done && b.reply) ? 'Ready: start when you are' : 'The presenter is getting ready…'
    if (held) return 'On hold'
    if (playing) return 'Speaking — tap the mic to interrupt'
    if (pending) {
      const doing = workingOn(msgs)
      return `${pending.kind === 'answer' ? 'Looking into your question' : 'Preparing the next part'}…${doing ? ` ${doing}` : ''}`
    }
    if (afterAnswer) return 'Back to the talk in a moment — ask again or hold'
    if (ended) return 'That was the talk. Ask anything.'
    return ''
  })()

  const history = beats.filter((b) => played.has(b.tag) && b.reply)

  return (
    <div className={shell(layout)}>
      <ScreenHeader tabs={layout === 'screen'}>
        <div className="flex min-h-8 items-center gap-1">
          <Button asChild variant="ghost" size="icon" className="-ml-2 size-9 shrink-0" aria-label="All QA sessions">
            <Link href="/qa">
              <ChevronLeftIcon />
            </Link>
          </Button>
          <h1 className="min-w-0 flex-1 truncate text-lg font-bold tracking-tight">{qa?.title ?? 'QA'}</h1>
          {presenter ? (
            <Link href={sessionHref(presenter)} className="shrink-0 text-xs text-muted-foreground hover:text-foreground">
              presenter
            </Link>
          ) : null}
        </div>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-4xl space-y-3 px-4 pt-3 pb-[max(1.5rem,env(safe-area-inset-bottom))]">
        <QaSlide slide={shown?.slide ?? null} />
        <p className="min-h-[3em] text-[15px] leading-relaxed">{shown?.speech ?? ''}</p>

        <section aria-label="Talk to the presenter" className="space-y-3 rounded-xl border bg-card p-3">
          <div className="flex items-center gap-3">
            {!started ? (
              <Button
                onClick={() => {
                  unlock()
                  setStarted(true)
                }}
                disabled={!presenter}
                className="h-12 rounded-full px-5"
              >
                <PlayIcon /> Start
              </Button>
            ) : canTranscribe ? (
              <button
                type="button"
                onClick={onMic}
                aria-label={mic.state === 'recording' ? 'Send the question' : 'Interrupt and ask'}
                className={cn(
                  'grid size-14 shrink-0 place-items-center rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring/50',
                  mic.state === 'recording' ? 'bg-status-error text-white' : 'bg-primary text-primary-foreground',
                )}
              >
                {mic.state === 'recording' ? <SquareIcon className="size-5" fill="currentColor" /> : mic.state === 'transcribing' ? <Loader2Icon className="size-6 animate-spin" /> : <MicIcon className="size-6" />}
              </button>
            ) : null}
            <p className="min-w-0 flex-1 text-sm text-muted-foreground">{line}</p>
            {started ? (
              <Button variant="outline" size="icon" className="size-10 shrink-0 rounded-full" onClick={onHold} aria-label={held ? 'Go on' : 'Hold'}>
                {held ? <PlayIcon /> : <PauseIcon />}
              </Button>
            ) : null}
          </div>
          {afterAnswer && !ended ? (
            <div className="flex gap-2">
              <Button size="sm" onClick={resume}>
                Continue the talk
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setAfterAnswer(false)}>
                Wait
              </Button>
            </div>
          ) : null}
          <form onSubmit={onType} className="flex gap-2">
            <Input value={typed} onChange={(e) => setTyped(e.target.value)} placeholder="…or type a question" aria-label="Your question" className="h-9" />
            <Button type="submit" size="icon" className="size-9 shrink-0" disabled={!typed.trim() || !presenter} aria-label="Ask">
              <SendIcon />
            </Button>
          </form>
        </section>

        {history.length ? (
          <details className="rounded-xl border bg-card p-3 text-sm">
            <summary className="cursor-pointer text-muted-foreground">What was said ({history.length})</summary>
            <div className="mt-2 space-y-2">
              {history.map((b) => (
                <div key={b.tag} className="space-y-0.5">
                  {b.question ? <p className="font-medium">You: {b.question}</p> : null}
                  <p className="text-muted-foreground">{parseBeat(b.reply ?? '').speech}</p>
                </div>
              ))}
            </div>
          </details>
        ) : null}
      </div>
    </div>
  )
}
