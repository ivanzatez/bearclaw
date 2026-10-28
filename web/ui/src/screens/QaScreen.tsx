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
import { goOnByItself, nextOrdered, onlySounds, resumeAt, statusSince, toSlide, type QaEvent, type QaSlide as QaSlideData } from '@/lib/qa'
import { allSessions, sessionHref } from '@/lib/sessions'
import { sessionTitle } from '@/lib/title'
import { cn } from '@/lib/utils'
import { formatElapsed } from '@/lib/voice'

/** `#/qa` (the QA sessions, and a new one) and `#/qa/<id>` (one, live). */
export function QaScreen({ layout = 'screen' }: { layout?: 'screen' | 'pane' }) {
  const [open, params] = useRoute('/qa/:id')
  return open ? <QaLive key={params.id} id={params.id} layout={layout} /> : <QaList layout={layout} />
}

const presenterOf = (sessions: Session[], qa: QaRecord | null) =>
  qa ? (sessions.find((s) => (qa.presenter.session && s.session_id === qa.presenter.session) || (qa.presenter.tmux && s.tmux_session === qa.presenter.tmux)) ?? null) : null
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
  const [lang, setLang] = useState<'hr' | 'en'>('en')
  // Who presents: the session itself (it keeps its context), or a new presenter started for it.
  const [mode, setMode] = useState<'self' | 'new'>('self')
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
      const qa = await api.createQa({ topic, lang, mode: src ? mode : 'new', source: src ? { host: src.host, session: src.session_id, title: sessionTitle(src), cwd: src.cwd ?? null, tmux: src.tmux_session ?? null } : null })
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
          <p className="text-sm text-muted-foreground">A session presents its work to you by voice, slide by slide. Interrupt it any time and ask: it answers from the real code.</p>
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
          {source ? (
            <div role="radiogroup" aria-label="Who presents" className="grid gap-2 sm:grid-cols-2">
              {(
                [
                  ['self', 'The session itself', 'It keeps its context, and goes back to its work when the QA ends.'],
                  ['new', 'A new presenter', 'A fresh session reads up on the work; it closes when the QA ends.'],
                ] as const
              ).map(([m, label, note]) => (
                <button
                  key={m}
                  type="button"
                  role="radio"
                  aria-checked={mode === m}
                  onClick={() => setMode(m)}
                  className={cn('rounded-lg border p-2.5 text-left text-sm', mode === m ? 'border-primary/50 bg-primary/10' : 'hover:bg-muted/50')}
                >
                  <span className="block font-medium">{label}</span>
                  <span className="block text-xs text-muted-foreground">{note}</span>
                </button>
              ))}
            </div>
          ) : null}
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
  // You were here before: the page picks up where you had heard up to (the server keeps it).
  const [resumable, setResumable] = useState(false)

  usePoller(async (signal) => {
    try {
      setQa(await api.qa(id, { signal }))
    } catch (err) {
      if ((err as Error)?.name !== 'AbortError') toast.error('QA session not found', { description: (err as Error)?.message })
    }
  }, 60_000)
  const presenter = presenterOf(allSessions(bearclaw), qa)
  const ended = events.some((e) => e.type === 'ended')

  // The live stream: long polls, back to back.
  useEffect(() => {
    const ctl = new AbortController()
    void (async () => {
      while (!ctl.signal.aborted) {
        try {
          const first = after.current === 0
          const r = await api.qaEvents(id, after.current, { signal: ctl.signal })
          if (first && r.heard) {
            const at = resumeAt(r.events, r.heard)
            setCursor(r.heard)
            setSlide(at.slide)
            setCaption(at.caption)
            setResumable(true)
          }
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
    // A tap with nothing said comes back as "[clicking]": no question, go on.
    if (!q || onlySounds(q)) {
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
    if (micState.current !== 'idle' || ended) return
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
  const slideNo = events.filter((e) => e.type === 'slide' && e.seq <= cursor).length
  const end = () => {
    audio.current?.pause()
    setHeld(true)
    api
      .qaEnd(id)
      .then((r) => toast.success(r.closes ? 'QA ended: the presenter is closing' : 'QA ended: the session went back to its work'))
      .catch((err) => toast.error('Not ended', { description: (err as Error)?.message }))
  }
  // Who has the floor, and the one line that says what happens next.
  const floor: Floor = (() => {
    if (ended) return { who: 'ready', line: qa?.mode === 'self' ? 'The QA is over: the session went back to its work' : 'The QA is over: the presenter closes' }
    if (!qa || (!presenter && !next)) return { who: 'wait', line: qa ? 'Starting the presenter…' : 'Loading…' }
    if (presenter?.status === 'waiting') return { who: 'wait', line: `The presenter is stuck on a prompt in its terminal${presenter.waiting_for ? `: ${presenter.waiting_for}` : ''}` }
    if (mic.state === 'recording') return { who: 'you', line: `You're talking (${formatElapsed(tick - (mic.startedAt ?? tick))})` }
    if (mic.state === 'transcribing') return { who: 'you', line: 'Got it' }
    if (!started) return resumable ? { who: 'ready', line: 'Pick up where you left off' } : next ? { who: 'ready', line: 'The presenter is ready' } : { who: 'wait', line: doing ?? 'The presenter is reading up' }
    if (held) return { who: 'ready', line: 'On hold' }
    if (speaking) return { who: 'presenter', line: 'Speaking' }
    if (listening) return { who: 'ready', line: listening.goOn ? 'Going on in a moment, unless you ask' : 'Your turn: ask anything' }
    return { who: 'thinking', line: doing ?? (lastQuestion && lastQuestion.seq > cursor ? 'Thinking about your question' : 'Preparing the next point') }
  })()

  const thread = events.filter((e) => (e.type === 'say' && e.seq <= cursor) || e.type === 'question')
  const threadEnd = useRef<HTMLDivElement>(null)
  useEffect(() => {
    threadEnd.current?.scrollIntoView({ block: 'nearest' })
  }, [thread.length])

  const mainButton = ended ? (
    <Button asChild variant="outline" className="h-12 rounded-full px-5">
      <Link href="/qa">All QA sessions</Link>
    </Button>
  ) : !started ? (
    <Button onClick={unlock} disabled={!presenter && !next} className="h-14 rounded-full px-6 text-base">
      <PlayIcon /> {resumable ? 'Resume' : 'Start'}
    </Button>
  ) : canTranscribe ? (
    <MicButton state={mic.state} who={floor.who} onPress={onPress} onRelease={onRelease} />
  ) : null

  return (
    <div className={shell(layout)}>
      <ScreenHeader tabs={layout === 'screen'} wide>
        <div className="flex min-h-8 items-center gap-1">
          <Button asChild variant="ghost" size="icon" className="-ml-2 size-9 shrink-0" aria-label="All QA sessions">
            <Link href="/qa">
              <ChevronLeftIcon />
            </Link>
          </Button>
          <h1 className="min-w-0 flex-1 truncate text-lg font-bold tracking-tight">{qa?.title ?? 'QA'}</h1>
          {presenter ? (
            <Link href={sessionHref(presenter)} className="shrink-0 px-2 text-xs text-muted-foreground hover:text-foreground">
              {qa?.mode === 'self' ? 'Open the session' : 'Open the presenter'}
            </Link>
          ) : null}
          {qa && !ended ? (
            <Button variant="ghost" size="sm" onClick={end} className="shrink-0 text-muted-foreground">
              End QA
            </Button>
          ) : null}
        </div>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-6xl px-4 pt-3 pb-[calc(7.5rem+env(safe-area-inset-bottom))] lg:pb-8">
        <div className="lg:grid lg:grid-cols-[minmax(0,1fr)_300px] lg:gap-6">
          <div className="min-w-0 space-y-4">
            <div className="space-y-1.5">
              <QaSlide key={slideNo} slide={slide} className="motion-safe:animate-in motion-safe:fade-in motion-safe:slide-in-from-bottom-2 motion-safe:duration-300" />
              {slideNo ? <p className="text-right text-xs text-dimmer tabular-nums">Slide {slideNo}</p> : null}
            </div>
            <p aria-live="polite" className={cn('min-h-[4.5rem] text-lg leading-snug text-balance transition-opacity duration-300 md:text-xl', speaking ? 'text-foreground' : 'text-muted-foreground')}>
              {caption || (started ? '' : 'Press Start and the presenter begins. Cut in whenever you like.')}
            </p>

            {/* The dock: on the phone it sits at the bottom, under the thumb. */}
            <div className="fixed inset-x-0 bottom-0 z-20 border-t bg-background/95 px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur-md lg:static lg:border-0 lg:bg-transparent lg:p-0 lg:backdrop-blur-none">
              <div className="mx-auto flex max-w-6xl items-center gap-4">
                {mainButton}
                <div className="min-w-0 flex-1">
                  <FloorLine floor={floor} />
                  {listening && !ended ? (
                    <div className="mt-2 flex gap-2">
                      <Button size="sm" onClick={goOn}>
                        Go on
                      </Button>
                      {listening.goOn ? (
                        <Button size="sm" variant="ghost" onClick={() => setListening({ ...listening, goOn: false })}>
                          Wait
                        </Button>
                      ) : null}
                    </div>
                  ) : canTranscribe && started ? (
                    <p className="mt-1.5 hidden flex-wrap gap-x-4 gap-y-1 text-xs text-dimmer lg:flex">
                      <span><Kbd>Space</Kbd> hold to talk</span>
                      <span><Kbd>M</Kbd> start and send</span>
                      <span><Kbd>Esc</Kbd> cancel</span>
                      <span><Kbd>C</Kbd> go on</span>
                    </p>
                  ) : null}
                </div>
                {started && !ended ? (
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-11 shrink-0 rounded-full"
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
            </div>
          </div>

          <aside aria-label="The conversation" className="mt-6 lg:sticky lg:top-20 lg:mt-0 lg:flex lg:max-h-[calc(100dvh-7rem)] lg:flex-col">
            <h2 className="mb-2 text-sm font-semibold text-muted-foreground">Conversation</h2>
            <div className="space-y-2 text-sm lg:min-h-0 lg:flex-1 lg:overflow-y-auto lg:pr-1">
              {thread.length ? (
                thread.map((e) =>
                  e.type === 'question' ? (
                    <p key={e.seq} className="ml-8 rounded-2xl rounded-br-md bg-primary/15 px-3 py-2 text-foreground">
                      {e.text}
                    </p>
                  ) : (
                    <p key={e.seq} className="mr-4 leading-relaxed text-muted-foreground">
                      {e.text}
                    </p>
                  ),
                )
              ) : (
                <p className="text-dimmer">What you both say shows up here.</p>
              )}
              <div ref={threadEnd} />
            </div>
            <form onSubmit={onType} className="mt-3 flex gap-2">
              <Input value={typed} onChange={(e) => setTyped(e.target.value)} placeholder="Type a question instead" aria-label="Your question" className="h-9" />
              <Button type="submit" size="icon" className="size-9 shrink-0" disabled={!typed.trim() || (!presenter && !next)} aria-label="Ask">
                <SendIcon />
              </Button>
            </form>
          </aside>
        </div>
      </div>
    </div>
  )
}

interface Floor {
  /** you: you are talking; presenter: it is speaking; thinking: it works; ready: it waits on you; wait: not yet. */
  who: 'you' | 'presenter' | 'thinking' | 'ready' | 'wait'
  line: string
}

function Kbd({ children }: { children: string }) {
  return <kbd className="rounded border bg-muted px-1 py-px font-mono text-[10px] text-muted-foreground">{children}</kbd>
}

/** The floor in one line: speaking bars, your timer, or what the presenter is doing. */
function FloorLine({ floor }: { floor: Floor }) {
  return (
    <p className={cn('flex min-w-0 items-center gap-2 text-sm', floor.who === 'you' ? 'text-status-error' : floor.who === 'presenter' ? 'text-primary' : 'text-muted-foreground')}>
      {floor.who === 'presenter' ? (
        <span className="flex h-4 shrink-0 items-end gap-[3px]" aria-hidden="true">
          {[0, 1, 2, 3].map((i) => (
            <span key={i} className="w-[3px] rounded-full bg-primary motion-safe:animate-[qa-bar_0.9s_ease-in-out_infinite]" style={{ height: '100%', animationDelay: `${i * 0.15}s` }} />
          ))}
        </span>
      ) : floor.who === 'thinking' || floor.who === 'wait' ? (
        <Loader2Icon className="size-3.5 shrink-0 motion-safe:animate-spin" aria-hidden="true" />
      ) : null}
      <span className="truncate">{floor.line}</span>
    </p>
  )
}

/** The mic: press and hold to talk, or tap to start and tap again to send. Its ring is the floor. */
function MicButton({ state, who, onPress, onRelease }: { state: string; who: Floor['who']; onPress: () => void; onRelease: () => void }) {
  const recording = state === 'recording'
  return (
    <button
      type="button"
      onPointerDown={(e) => {
        e.preventDefault()
        onPress()
      }}
      onPointerUp={onRelease}
      onPointerLeave={onRelease}
      onContextMenu={(e) => e.preventDefault()}
      aria-label={recording ? 'Send the question' : 'Hold to talk'}
      className={cn(
        'relative grid size-16 shrink-0 touch-none place-items-center rounded-full outline-none select-none focus-visible:ring-2 focus-visible:ring-ring/50 lg:size-[4.5rem]',
        recording ? 'bg-status-error text-white' : 'bg-primary text-primary-foreground',
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          'pointer-events-none absolute -inset-1.5 rounded-full border-2',
          recording ? 'border-status-error/60 motion-safe:animate-ping' : who === 'presenter' ? 'border-primary/40' : 'border-transparent',
        )}
      />
      {recording ? <SquareIcon className="size-5" fill="currentColor" /> : state === 'transcribing' ? <Loader2Icon className="size-6 animate-spin" /> : <MicIcon className="size-7" />}
    </button>
  )
}
