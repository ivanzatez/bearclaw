import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { ChevronLeftIcon, ClapperboardIcon, Loader2Icon, MicIcon, PauseIcon, PlayIcon, SendIcon, SquareIcon } from 'lucide-react'
import { toast } from 'sonner'
import { Link, useRoute } from 'wouter'

import { ApiError, api } from '@/api/client'
import type { Video, VideoSlide, VideoSummary } from '@/api/types'
import { ScreenHeader } from '@/components/ScreenHeader'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useNow } from '@/hooks/useNow'
import { usePoller } from '@/hooks/usePoller'
import { useSettings } from '@/hooks/useSettings'
import { useSpeechPlayer } from '@/hooks/useSpeechPlayer'
import { useVoice } from '@/hooks/useVoice'
import { relTime } from '@/lib/format'
import { cn } from '@/lib/utils'
import { formatElapsed } from '@/lib/voice'
import { answerTo, askMessage, fmtTime, lineAt, questionTag, sceneAt, slideSince } from '@/lib/videos'

/** `#/videos` (the list) and `#/videos/<id>` (one video). */
export function VideosScreen({ layout = 'screen' }: { layout?: 'screen' | 'pane' }) {
  const [open, params] = useRoute('/videos/:id')
  return open ? <VideoPlayer key={params.id} id={params.id} layout={layout} /> : <VideoList layout={layout} />
}

function VideoList({ layout }: { layout: 'screen' | 'pane' }) {
  const now = useNow(30_000)
  const [videos, setVideos] = useState<VideoSummary[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  usePoller(async (signal) => {
    try {
      setVideos((await api.videos({ signal })).videos)
      setError(null)
    } catch (err) {
      if ((err as Error)?.name !== 'AbortError') setError(err instanceof ApiError && err.status === 404 ? 'This server predates videos: update bearclaw and restart its web server.' : (err as Error)?.message || 'could not load videos')
    }
  }, 30_000)
  return (
    <div className={cn('flex flex-col bg-background', layout === 'screen' ? 'min-h-app' : 'min-h-0 flex-1 overflow-y-auto')}>
      <ScreenHeader tabs={layout === 'screen'}>
        <h1 className="flex min-h-8 items-center text-xl font-bold tracking-tight">Videos</h1>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-3xl space-y-2 px-4 pt-3 pb-[max(1.5rem,env(safe-area-inset-bottom))]">
        {error ? <p className="text-sm text-status-waiting">{error}</p> : null}
        {videos && !videos.length ? (
          <p className="py-8 text-center text-sm text-muted-foreground">No videos yet. A session makes one in ~/.local/state/bearclaw/videos.</p>
        ) : null}
        {(videos ?? []).map((v) => (
          <Link key={v.id} href={`/videos/${v.id}`} className="flex items-center gap-3 rounded-lg border bg-card p-3 text-sm hover:border-primary/40 hover:bg-muted/40">
            <ClapperboardIcon className="size-5 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1">
              <span className="block truncate font-medium">{v.title}</span>
              <span className="block text-xs text-muted-foreground tabular-nums">
                {fmtTime(v.duration)} · {v.scenes} scenes · {relTime(v.updatedAt, now) || 'just now'}
              </span>
            </span>
          </Link>
        ))}
      </div>
    </div>
  )
}

interface Turn {
  tag: string
  at: number
  question: string
  sentAt: number
  answer: string | null
  slide: VideoSlide | null
}

/** Post a message to the player page (it only listens to `frame` and `slide`). */
function post(frame: HTMLIFrameElement | null, msg: unknown) {
  frame?.contentWindow?.postMessage(msg, '*')
}

function VideoPlayer({ id, layout }: { id: string; layout: 'screen' | 'pane' }) {
  const { self, voice: canTranscribe } = useSettings()
  const speech = useSpeechPlayer()
  const [video, setVideo] = useState<Video | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [playing, setPlaying] = useState(false)
  const [t, setT] = useState(0)
  const [typed, setTyped] = useState('')
  const [turns, setTurns] = useState<Turn[]>([])
  const frameRef = useRef<HTMLIFrameElement>(null)
  const audio = useRef<HTMLAudioElement | null>(null)
  const ready = useRef(false)
  const tRef = useRef(0)
  const waiting = turns.find((x) => x.answer === null) ?? null

  // The video (and, while a question waits, its slides).
  usePoller(
    async (signal) => {
      try {
        setVideo(await api.video(id, { signal }))
        setError(null)
      } catch (err) {
        if ((err as Error)?.name !== 'AbortError') setError((err as Error)?.message || 'could not load the video')
      }
    },
    waiting ? 2000 : 60_000,
  )

  // The narration is the clock: loaded as a Blob, so seeking needs no range requests.
  useEffect(() => {
    const controller = new AbortController()
    let url: string | null = null
    const a = new Audio()
    a.preload = 'auto'
    a.onended = () => setPlaying(false)
    audio.current = a
    api
      .videoAudio(id, { signal: controller.signal })
      .then((blob) => {
        url = URL.createObjectURL(blob)
        a.src = url
      })
      .catch((err) => {
        if ((err as Error)?.name !== 'AbortError') setError(`No narration: ${(err as Error)?.message}`)
      })
    return () => {
      controller.abort()
      a.pause()
      if (url) URL.revokeObjectURL(url)
    }
  }, [id])

  const draw = useCallback((time: number) => {
    tRef.current = time
    if (ready.current) post(frameRef.current, { type: 'frame', t: time })
  }, [])

  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      if (e.source !== frameRef.current?.contentWindow || (e.data as { type?: string })?.type !== 'ready') return
      ready.current = true
      post(frameRef.current, { type: 'frame', t: tRef.current })
    }
    window.addEventListener('message', onMsg)
    return () => window.removeEventListener('message', onMsg)
  }, [])

  // While playing: draw every frame, update the controls ~8 times a second.
  useEffect(() => {
    if (!playing) return
    let raf = 0
    let last = 0
    const tick = (now: number) => {
      const a = audio.current
      if (a) {
        draw(a.currentTime)
        if (now - last > 120) {
          last = now
          setT(a.currentTime)
        }
      }
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [playing, draw])

  const duration = video?.duration ?? 0
  const seek = (time: number) => {
    const v = Math.min(Math.max(0, time), duration || 0)
    if (audio.current) audio.current.currentTime = v
    setT(v)
    draw(v)
  }
  const pause = () => {
    audio.current?.pause()
    setPlaying(false)
  }
  const play = async () => {
    const a = audio.current
    if (!a) return
    speech.stop()
    post(frameRef.current, { type: 'slide', slide: null })
    if (a.currentTime >= duration - 0.05) a.currentTime = 0
    try {
      await a.play()
      setPlaying(true)
    } catch (err) {
      toast.error('Could not play the video', { description: (err as Error)?.message })
    }
  }

  // Ask: the question goes to the session that made the video, with where you paused.
  const host = video?.host ?? self
  const ask = async (question: string) => {
    const q = question.trim()
    if (!q || !video) return
    if (!video.session || !host) return toast.error('This video has no session to ask')
    pause()
    speech.unlock()
    const at = tRef.current
    const scene = video.scenes[sceneAt(video.scenes, at)]
    const tag = questionTag()
    const sentAt = Date.now()
    const text = askMessage({ title: video.title, scene, line: lineAt(scene, at), t: at, question: q, slidesDir: video.slidesDir, tag })
    try {
      await api.send(host, video.session, text)
      setTurns((list) => [{ tag, at, question: q, sentAt, answer: null, slide: null }, ...list])
      setTyped('')
    } catch (err) {
      toast.error(err instanceof ApiError && err.status === 404 ? 'The session that made this video is not running' : 'Question not sent', { description: (err as Error)?.message })
    }
  }

  // The answer: the session's reply after the question, read aloud; a slide it wrote goes on the film.
  usePoller(
    async (signal) => {
      if (!waiting || !video?.session || !host) return
      const r = await api.messages(host, video.session, 40, { signal })
      const answer = answerTo(r.messages, waiting.tag, r.status)
      if (!answer) return
      const slide = slideSince((await api.video(id, { signal })).slides, waiting.sentAt - 1000)
      setTurns((list) => list.map((x) => (x.tag === waiting.tag ? { ...x, answer, slide } : x)))
      if (slide) post(frameRef.current, { type: 'slide', slide: { ...slide, eyebrow: `Answer · ${fmtTime(waiting.at)}` } })
      void speech.speak(answer, `video:${waiting.tag}`)
    },
    1500,
    { enabled: Boolean(waiting) },
  )
  // A slide written before the reply goes up at once.
  const pendingSlide = useMemo(() => (waiting && video ? slideSince(video.slides, waiting.sentAt - 1000) : null), [waiting, video])
  useEffect(() => {
    if (pendingSlide && waiting) post(frameRef.current, { type: 'slide', slide: { ...pendingSlide, eyebrow: `Answer · ${fmtTime(waiting.at)}` } })
  }, [pendingSlide, waiting])

  const mic = useVoice({ onText: (text) => void ask(text), hint: video ? `${video.title}, ${video.scenes.map((s) => s.name).join(', ')}` : '' })
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (mic.state !== 'recording') return
    const timer = window.setInterval(() => setNow(Date.now()), 500)
    return () => window.clearInterval(timer)
  }, [mic.state])
  const onMic = () => {
    if (speech.state !== 'idle') speech.stop()
    if (mic.state === 'recording') return mic.stop()
    if (mic.state === 'transcribing') return mic.cancel()
    pause()
    speech.unlock()
    void mic.start()
  }
  const onType = (e: FormEvent) => {
    e.preventDefault()
    void ask(typed)
  }

  const sceneIdx = video ? sceneAt(video.scenes, t) : 0
  const scene = video?.scenes[sceneIdx]
  const line = lineAt(scene, t)
  const status =
    mic.state === 'recording'
      ? `Listening… ${formatElapsed(now - (mic.startedAt ?? now))} — tap to send`
      : mic.state === 'transcribing'
        ? 'Transcribing…'
        : waiting
          ? 'The session is answering…'
          : speech.state === 'loading'
            ? 'Preparing the answer…'
            : speech.state === 'playing'
              ? 'Answering — tap the mic to interrupt and ask'
              : canTranscribe
                ? 'Pause anytime: tap the mic and ask'
                : 'Pause anytime and type a question'

  return (
    <div className={cn('flex flex-col bg-background', layout === 'screen' ? 'min-h-app' : 'min-h-0 flex-1 overflow-y-auto')}>
      <ScreenHeader tabs={layout === 'screen'}>
        <div className="flex min-h-8 items-center gap-1">
          <Button asChild variant="ghost" size="icon" className="-ml-2 size-9 shrink-0" aria-label="All videos">
            <Link href="/videos">
              <ChevronLeftIcon />
            </Link>
          </Button>
          <h1 className="min-w-0 flex-1 truncate text-lg font-bold tracking-tight">{video?.title ?? 'Video'}</h1>
        </div>
      </ScreenHeader>
      <div className="mx-auto w-full max-w-3xl space-y-3 px-4 pt-3 pb-[max(1.5rem,env(safe-area-inset-bottom))]">
        {error ? <p className="text-sm text-status-waiting">{error}</p> : null}
        <div className="relative aspect-video w-full overflow-hidden rounded-xl bg-[#0f1020]">
          <iframe ref={frameRef} src={api.videoPlayerUrl(id)} sandbox="allow-scripts" title={video?.title ?? 'Video'} className="absolute inset-0 size-full border-0" />
          <button type="button" aria-label={playing ? 'Pause' : 'Play'} onClick={() => (playing ? pause() : void play())} className="absolute inset-0 grid place-items-center outline-none focus-visible:ring-2 focus-visible:ring-ring/50">
            {!playing ? (
              <span className="grid size-16 place-items-center rounded-full bg-black/45 text-white">
                <PlayIcon className="ml-1 size-7" fill="currentColor" />
              </span>
            ) : null}
          </button>
        </div>

        <div className="flex items-center gap-2">
          <Button size="sm" onClick={() => (playing ? pause() : void play())} disabled={!video}>
            {playing ? <PauseIcon /> : <PlayIcon />}
            {playing ? 'Pause' : t > 0.05 ? 'Resume' : 'Play'}
          </Button>
          <span className="shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
            {fmtTime(t)} / {fmtTime(duration)}
          </span>
          <input type="range" min={0} max={duration || 1} step={0.05} value={t} onChange={(e) => seek(Number(e.target.value))} aria-label="Position in the video" className="h-7 min-w-0 flex-1 accent-primary" />
        </div>

        <p className="min-h-[3em] text-[15px] leading-relaxed">{line?.text ?? ''}</p>

        <div className="flex gap-1.5 overflow-x-auto pb-1">
          {(video?.scenes ?? []).map((s, i) => (
            <button
              key={s.id}
              type="button"
              onClick={() => seek(s.start + 0.01)}
              className={cn('shrink-0 rounded-full border px-2.5 py-1 text-xs whitespace-nowrap', i === sceneIdx ? 'border-primary/50 bg-primary/10 text-foreground' : 'text-muted-foreground hover:bg-muted/60')}
            >
              <span className="mr-1 font-mono tabular-nums">{fmtTime(s.start)}</span>
              {s.name}
            </button>
          ))}
        </div>

        <section aria-label="Ask about the video" className="space-y-3 rounded-xl border bg-card p-3">
          <div className="flex items-center gap-3">
            {canTranscribe ? (
              <button
                type="button"
                onClick={onMic}
                aria-label={mic.state === 'recording' ? 'Send the question' : 'Ask by voice'}
                className={cn(
                  'grid size-14 shrink-0 place-items-center rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring/50',
                  mic.state === 'recording' ? 'bg-status-error text-white' : 'bg-primary text-primary-foreground',
                )}
              >
                {mic.state === 'recording' ? <SquareIcon className="size-5" fill="currentColor" /> : mic.state === 'transcribing' ? <Loader2Icon className="size-6 animate-spin" /> : <MicIcon className="size-6" />}
              </button>
            ) : null}
            <p className="min-w-0 flex-1 text-sm text-muted-foreground">{status}</p>
          </div>
          <form onSubmit={onType} className="flex gap-2">
            <Input value={typed} onChange={(e) => setTyped(e.target.value)} placeholder="…or type a question" aria-label="Your question" className="h-9" />
            <Button type="submit" size="icon" className="size-9 shrink-0" disabled={!typed.trim() || !video} aria-label="Ask">
              <SendIcon />
            </Button>
          </form>
          {turns.map((x) => (
            <div key={x.tag} className="space-y-1 border-t pt-3 text-sm">
              <p className="font-medium">
                <button type="button" onClick={() => seek(x.at)} className="mr-2 font-mono text-xs text-muted-foreground tabular-nums hover:text-foreground">
                  {fmtTime(x.at)}
                </button>
                {x.question}
              </p>
              {x.answer ? (
                <p className="whitespace-pre-wrap text-muted-foreground">{x.answer}</p>
              ) : (
                <p className="flex items-center gap-1.5 text-muted-foreground">
                  <Loader2Icon className="size-3.5 animate-spin" /> The session is answering…
                </p>
              )}
              {x.slide ? <p className="text-xs text-dimmer">Slide on the video: {x.slide.title}</p> : null}
            </div>
          ))}
        </section>
      </div>
    </div>
  )
}
