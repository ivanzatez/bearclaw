import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { ChevronLeftIcon, Loader2Icon, MicIcon, PauseIcon, PlayIcon, PresentationIcon, SendIcon, SquareIcon } from 'lucide-react'
import { toast } from 'sonner'
import { Link, useLocation, useRoute } from 'wouter'

import { ApiError, api } from '@/api/client'
import type { QaRecord, Session } from '@/api/types'
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
import { goOnByItself, nextOrdered, statusSince, toSlide, type QaEvent, type QaSlide as QaSlideData } from '@/lib/qa'
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
      // The server writes the presenter's workspace and starts it there.
      const qa = await api.createQa({ topic, lang, source: src ? { host: src.host, session: src.session_id, title: sessionTitle(src), cwd: src.cwd ?? null } : null })
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

// 50 ms of silence: played inside a tap, it lets this element play what comes later (iOS).
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

/** After an answer, back to the talk when you stay quiet this long. */
const GO_ON_MS = 5000
/** Held longer than this, the mic button (or Space) is push-to-talk: letting go sends. */
const HOLD_MS = 350

const typingTarget = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))

function QaLive({ id, layout }: { id: string; layout: 'screen' | 'pane' }) {
  const { voice: canTranscribe } = useSettings()
  const { bearclaw } = useBearclaw()
  const [qa, setQa] = useState<QaRecord | null>(null)
  const [events, setEvents] = useState<QaEvent[]>([])
  const [started, setStarted] = useState(false)
  const [held, setHeld] = useState(false)
  // The last ordered event played; what comes after it waits its turn.
  const [cursor, setCursor] = useState(0)
  const [speaking, setSpeaking] = useState<string | null>(null)
  const [caption, setCaption] = useState('')
  const [slide, setSlide] = useState<QaSlideData | null>(null)
  const [listening, setListening] = useState<{ seq: number; goOn: boolean } | null>(null)
  const [typed, setTyped] = useState('')
  const audio = useRef<HTMLAudioElement | null>(null)
  const after = useRef(0)
  const holdStart = useRef(0)

  usePoller(async (signal) => {
    try {
      setQa(await api.qa(id, { signal }))
    } catch (err) {
      if ((err as Error)?.name !== 'AbortError') toast.error('QA session not found', { description: (err as Error)?.message })
    }
  }, 60_000)
  const presenter = presenterOf(allSessions(bearclaw), qa)

  // The live stream: long polls, back to back.
  useEffect(() => {
    const ctl = new AbortController()
    void (async () => {
      while (!ctl.signal.aborted) {
        try {
          const r = await api.qaEvents(id, after.current, { signal: ctl.signal })
          if (r.events.length) {
            after.current = r.seq
            setEvents((list) => [...list, ...r.events].slice(-500))
          } else if (r.seq < after.current) after.current = r.seq
        } catch (err) {
          if (ctl.signal.aborted) return
          void err
          await new Promise((res) => setTimeout(res, 2000))
        }
      }
    })()
    return () => ctl.abort()
  }, [id])

  useEffect(() => {
    const a = new Audio()
    a.preload = 'auto'
    audio.current = a
    return () => a.pause()
  }, [])

  // Play the stream in order: a slide goes up, a sentence is spoken, then the next.
  const next = nextOrdered(events, cursor)
  const stepRef = useRef<(e: QaEvent) => void>(() => {})
  const step = useCallback((e: QaEvent) => stepRef.current(e), [])
  useEffect(() => {
    if (!started || held || speaking || !next) return
    // Stepped outside the effect's own pass (a render may land in between: then this one is off).
    let live = true
    void Promise.resolve().then(() => live && step(next))
    return () => {
      live = false
    }
  }, [started, held, speaking, next, step])

  function stepImpl(next: QaEvent) {
    if (next.type === 'slide') {
      setSlide(toSlide(next.slide))
      setCursor(next.seq)
      return
    }
    if (next.type === 'listening') {
      setListening({ seq: next.seq, goOn: goOnByItself(events, next.seq) })
      setCursor(next.seq)
      return
    }
    const a = audio.current
    if (!a || !next.sid) return setCursor(next.seq)
    const sid = next.sid
    setSpeaking(sid)
    setListening(null)
    setCaption(next.text ?? '')
    const done = () => {
      if (a.dataset.sid !== sid) return
      a.dataset.sid = ''
      setSpeaking(null)
      setCursor(next.seq)
      api.qaPlayed(id, sid).catch(() => {})
    }
    a.dataset.sid = sid
    a.onended = done
    a.onerror = done
    a.src = api.qaAudioUrl(id, sid)
    a.play().catch(done)
  }
  useEffect(() => {
    stepRef.current = stepImpl
  })

  // After an answer the presenter asks whether to go on: it does, unless you speak.
  const goOn = useCallback(() => {
    setListening(null)
    api.qaContinue(id).catch(() => {})
  }, [id])
  useEffect(() => {
    if (!listening?.goOn || held) return
    const t = window.setTimeout(goOn, GO_ON_MS)
    return () => window.clearTimeout(t)
  }, [listening, held, goOn])

  const unlock = () => {
    const a = audio.current
    if (a && !a.src) {
      a.src = silentWav()
      a.play().catch(() => {})
    }
    setStarted(true)
  }

  /** You start talking: the voice stops, what was queued is dropped, the presenter holds. */
  const cutIn = () => {
    const a = audio.current
    if (a) {
      a.pause()
      a.dataset.sid = ''
    }
    setSpeaking(null)
    setListening(null)
    setCursor(after.current)
    api.qaInterrupt(id).catch(() => {})
  }

  const ask = (text: string) => {
    const q = text.trim()
    if (!q) {
      api.qaContinue(id).catch(() => {})
      return
    }
    api.qaAsk(id, q).catch((err) => toast.error('Question not sent', { description: (err as Error)?.message }))
  }

  const mic = useVoice({ onText: ask, hint: qa?.title ?? '' })
  const micState = useRef(mic.state)
  useEffect(() => {
    micState.current = mic.state
  }, [mic.state])

  const talk = () => {
    if (micState.current !== 'idle') return
    unlock()
    cutIn()
    holdStart.current = Date.now()
    void mic.start()
  }
  const send = () => {
    if (micState.current === 'recording') mic.stop()
  }
  const cancel = () => {
    if (micState.current === 'idle') return
    mic.cancel()
    api.qaContinue(id).catch(() => {})
  }
  // The button: press and hold to talk (let go sends), or tap to start and tap again to send.
  const onPress = () => (micState.current === 'recording' ? send() : talk())
  const onRelease = () => {
    if (micState.current === 'recording' && Date.now() - holdStart.current > HOLD_MS) send()
  }

  // Keys: hold Space to talk, M to start / send, Esc to cancel, C to go on.
  const keys = useRef({ talk, send, cancel, goOn, onPress })
  useEffect(() => {
    keys.current = { talk, send, cancel, goOn, onPress }
  })
  useEffect(() => {
    if (!canTranscribe) return
    const down = (e: KeyboardEvent) => {
      if (typingTarget(e.target) || e.metaKey || e.ctrlKey || e.altKey) return
      if (e.code === 'Space') {
        e.preventDefault()
        if (!e.repeat) keys.current.talk()
      } else if (e.key === 'm' || e.key === 'M') keys.current.onPress()
      else if (e.key === 'Escape') keys.current.cancel()
      else if (e.key === 'c' || e.key === 'C') keys.current.goOn()
    }
    const up = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !typingTarget(e.target)) keys.current.send()
    }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
    }
  }, [canTranscribe])

  const onType = (e: FormEvent) => {
    e.preventDefault()
    unlock()
    cutIn()
    ask(typed)
    setTyped('')
  }

  const tick = useNow(500)
  const doing = statusSince(events, cursor)
  const lastQuestion = events.findLast((e) => e.type === 'question')
  const line = (() => {
    if (!qa) return 'Loading…'
    if (!presenter) return 'Starting the presenter…'
    if (presenter.status === 'waiting') return `The presenter waits on you in its terminal${presenter.waiting_for ? `: ${presenter.waiting_for}` : ''}`
    if (mic.state === 'recording') return `Listening… ${formatElapsed(tick - (mic.startedAt ?? tick))} — let go of Space (or tap) to send, Esc cancels`
    if (mic.state === 'transcribing') return 'Got it…'
    if (!started) return next ? 'Ready: start when you are' : `The presenter is getting ready…${doing ? ` ${doing}` : ''}`
    if (held) return 'On hold'
    if (speaking) return canTranscribe ? 'Hold Space or the mic to cut in' : ''
    if (listening) return listening.goOn ? 'Ask something else, or it goes on in a moment (C: now)' : 'Your turn: ask anything'
    return doing ?? (lastQuestion && lastQuestion.seq > cursor ? 'Thinking about your question…' : 'Thinking…')
  })()

  const said = events.filter((e) => (e.type === 'say' && e.seq <= cursor) || e.type === 'question')

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
        <QaSlide slide={slide} />
        <p className={cn('min-h-[3em] text-[15px] leading-relaxed transition-opacity', speaking ? 'opacity-100' : 'opacity-60')}>{caption}</p>

        <section aria-label="Talk to the presenter" className="space-y-3 rounded-xl border bg-card p-3">
          <div className="flex items-center gap-3">
            {!started ? (
              <Button onClick={unlock} disabled={!presenter} className="h-12 rounded-full px-5">
                <PlayIcon /> Start
              </Button>
            ) : canTranscribe ? (
              <button
                type="button"
                onPointerDown={(e) => {
                  e.preventDefault()
                  onPress()
                }}
                onPointerUp={onRelease}
                onPointerLeave={onRelease}
                onContextMenu={(e) => e.preventDefault()}
                aria-label={mic.state === 'recording' ? 'Send the question' : 'Hold to talk'}
                className={cn(
                  'grid size-16 shrink-0 touch-none place-items-center rounded-full outline-none select-none focus-visible:ring-2 focus-visible:ring-ring/50',
                  mic.state === 'recording' ? 'scale-110 bg-status-error text-white' : 'bg-primary text-primary-foreground',
                )}
              >
                {mic.state === 'recording' ? <SquareIcon className="size-5" fill="currentColor" /> : mic.state === 'transcribing' ? <Loader2Icon className="size-6 animate-spin" /> : <MicIcon className="size-7" />}
              </button>
            ) : null}
            <p className="min-w-0 flex-1 text-sm text-muted-foreground">{line}</p>
            {started ? (
              <Button
                variant="outline"
                size="icon"
                className="size-10 shrink-0 rounded-full"
                onClick={() => {
                  setHeld((h) => !h)
                  if (held) void audio.current?.play().catch(() => {})
                  else audio.current?.pause()
                }}
                aria-label={held ? 'Go on' : 'Hold'}
              >
                {held ? <PlayIcon /> : <PauseIcon />}
              </Button>
            ) : null}
          </div>
          {listening ? (
            <div className="flex gap-2">
              <Button size="sm" onClick={goOn}>
                Go on
              </Button>
              {listening.goOn ? (
                <Button size="sm" variant="ghost" onClick={() => setListening({ ...listening, goOn: false })}>
                  Wait
                </Button>
              ) : null}
            </div>
          ) : null}
          <form onSubmit={onType} className="flex gap-2">
            <Input value={typed} onChange={(e) => setTyped(e.target.value)} placeholder="…or type a question" aria-label="Your question" className="h-9" />
            <Button type="submit" size="icon" className="size-9 shrink-0" disabled={!typed.trim() || !presenter} aria-label="Ask">
              <SendIcon />
            </Button>
          </form>
          {canTranscribe ? <p className="hidden text-xs text-dimmer md:block">Hold Space to talk · M starts and sends · Esc cancels · C goes on</p> : null}
        </section>

        {said.length ? (
          <details className="rounded-xl border bg-card p-3 text-sm">
            <summary className="cursor-pointer text-muted-foreground">What was said ({said.length})</summary>
            <div className="mt-2 space-y-1.5">
              {said.map((e) => (
                <p key={e.seq} className={e.type === 'question' ? 'font-medium' : 'text-muted-foreground'}>
                  {e.type === 'question' ? `You: ${e.text}` : e.text}
                </p>
              ))}
            </div>
          </details>
        ) : null}
      </div>
    </div>
  )
}
