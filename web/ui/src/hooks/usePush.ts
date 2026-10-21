import { useCallback, useEffect, useState } from 'react'

import { ApiError, api } from '@/api/client'
import type { PushDevice } from '@/api/types'
import { useSettings } from '@/hooks/useSettings'
import { browserPushEnv, currentSubscription, keyBytes, pushSupport, type PushSupport } from '@/lib/push'

export type PushPrefs = { events?: Partial<NonNullable<PushDevice['events']>>; awayOnly?: boolean }

export interface PushState {
  /** `off`: this server sends no notifications. */
  support: PushSupport | 'off'
  /** null while loading. */
  device: PushDevice | null
  busy: boolean
  error: string | null
  enable: () => Promise<void>
  disable: () => Promise<void>
  update: (prefs: PushPrefs) => Promise<void>
  test: () => Promise<boolean>
}

const message = (err: unknown) => (err instanceof ApiError || err instanceof Error ? err.message : String(err))

function sameKey(sub: PushSubscription, key: Uint8Array): boolean {
  const have = sub.options?.applicationServerKey
  if (!have) return true
  const bytes = new Uint8Array(have)
  return bytes.length === key.length && bytes.every((b, i) => b === key[i])
}

/** Settings → Notifications: this device's subscription and its preferences on the server. */
export function usePush(): PushState {
  const { pushKey, loaded } = useSettings()
  const support: PushState['support'] = !loaded || pushKey ? pushSupport(browserPushEnv()) : 'off'
  const [device, setDevice] = useState<PushDevice | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (support !== 'ok' || !pushKey) return
    let cancelled = false
    currentSubscription()
      .then((sub) => (sub ? api.pushDevice(sub.endpoint) : { subscribed: false }))
      .then((d) => !cancelled && setDevice(d))
      .catch((err) => !cancelled && (setDevice({ subscribed: false }), setError(message(err))))
    return () => {
      cancelled = true
    }
  }, [support, pushKey])

  const step = useCallback(async (work: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await work()
    } catch (err) {
      setError(message(err))
    } finally {
      setBusy(false)
    }
  }, [])

  const enable = useCallback(
    () =>
      step(async () => {
        if (!pushKey) return
        // First, inside the tap: iOS only asks from a user gesture.
        const permission = await Notification.requestPermission()
        if (permission !== 'granted') {
          throw new Error(permission === 'denied' ? 'Notifications are blocked for bearclaw: allow them in the phone’s Settings → Notifications' : 'Notifications were not allowed')
        }
        const reg = await navigator.serviceWorker.register('/sw.js')
        await navigator.serviceWorker.ready
        const key = keyBytes(pushKey)
        let sub = await reg.pushManager.getSubscription()
        if (sub && !sameKey(sub, key)) {
          await sub.unsubscribe()
          sub = null
        }
        sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key })
        setDevice(await api.pushSubscribe({ subscription: sub.toJSON() }))
      }),
    [pushKey, step],
  )

  const disable = useCallback(
    () =>
      step(async () => {
        const sub = await currentSubscription()
        if (sub) {
          await api.pushUnsubscribe(sub.endpoint).catch(() => {})
          await sub.unsubscribe()
        }
        setDevice({ subscribed: false })
      }),
    [step],
  )

  const update = useCallback(
    (prefs: PushPrefs) =>
      step(async () => {
        const sub = await currentSubscription()
        if (!sub) throw new Error('This device is not subscribed any more: turn notifications on again')
        setDevice((d) => (d?.events ? { ...d, ...prefs, events: { ...d.events, ...prefs.events } } : d))
        setDevice(await api.pushSubscribe({ subscription: sub.toJSON(), ...prefs }))
      }),
    [step],
  )

  const test = useCallback(async () => {
    let ok = false
    await step(async () => {
      const sub = await currentSubscription()
      if (!sub) throw new Error('This device is not subscribed')
      await api.pushTest(sub.endpoint)
      ok = true
    })
    return ok
  }, [step])

  return { support, device, busy, error, enable, disable, update, test }
}

/**
 * While the app is on screen, tell the server (every 30 s, and on every visibility change), so a
 * subscribed device is not notified about what it is already looking at.
 */
export function usePushPresence(): void {
  const { pushKey } = useSettings()
  useEffect(() => {
    if (!pushKey || !('Notification' in window)) return
    let endpoint: string | null = null
    let stopped = false
    const ping = async () => {
      if (Notification.permission !== 'granted') return
      endpoint ??= (await currentSubscription().catch(() => null))?.endpoint ?? null
      if (!endpoint || stopped) return
      const visible = document.visibilityState === 'visible'
      api.pushPresence(endpoint, visible, { keepalive: !visible }).catch(() => {})
    }
    void ping()
    const timer = setInterval(() => document.visibilityState === 'visible' && void ping(), 30_000)
    document.addEventListener('visibilitychange', ping)
    return () => {
      stopped = true
      clearInterval(timer)
      document.removeEventListener('visibilitychange', ping)
    }
  }, [pushKey])
}
