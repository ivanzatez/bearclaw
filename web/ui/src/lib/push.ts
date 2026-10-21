// Push notifications in the browser: can this device get them, the service worker, and the
// device's subscription. The server side is lib/push.mjs (ARCHITECTURE.md → Push notifications).

export type PushSupport = 'ok' | 'install' | 'insecure' | 'unsupported'

export interface PushEnv {
  secure: boolean
  serviceWorker: boolean
  pushManager: boolean
  notification: boolean
  standalone: boolean
  ios: boolean
}

/**
 * Whether this page can subscribe. iPhone and iPad only offer push to an app added to the Home
 * Screen and opened from there (`install`); every browser needs HTTPS (`insecure`).
 */
export function pushSupport(env: PushEnv): PushSupport {
  if (!env.secure) return 'insecure'
  if (env.serviceWorker && env.pushManager && env.notification) return 'ok'
  if (env.ios && !env.standalone) return 'install'
  return 'unsupported'
}

export function browserPushEnv(): PushEnv {
  const nav = navigator as Navigator & { standalone?: boolean }
  return {
    secure: window.isSecureContext,
    serviceWorker: 'serviceWorker' in navigator,
    pushManager: 'PushManager' in window,
    notification: 'Notification' in window,
    standalone: nav.standalone === true || window.matchMedia('(display-mode: standalone)').matches,
    ios: /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1),
  }
}

/** A base64url VAPID public key as the bytes `pushManager.subscribe` takes. */
export function keyBytes(base64url: string): Uint8Array<ArrayBuffer> {
  const b64 = base64url.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (base64url.length % 4)) % 4)
  const raw = atob(b64)
  const out = new Uint8Array(new ArrayBuffer(raw.length))
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i)
  return out
}

/** The hash route a notification asks to open (`/#/s/<host>/<id>`), or null for anything else. */
export function openRoute(url: unknown, origin: string): string | null {
  if (typeof url !== 'string') return null
  try {
    const u = new URL(url, origin)
    if (u.origin !== origin || !u.hash.startsWith('#/')) return null
    return u.hash.slice(1)
  } catch {
    return null
  }
}

/**
 * Register the service worker (push only) and follow its "open this session" messages: a tap on
 * a notification while the app is open switches the route instead of reloading the page.
 */
export function installServiceWorker(navigate: (route: string) => void = (r) => (window.location.hash = r)): void {
  if (!('serviceWorker' in navigator) || !window.isSecureContext) return
  navigator.serviceWorker.register('/sw.js').catch(() => {})
  navigator.serviceWorker.addEventListener('message', (e: MessageEvent) => {
    if (e.data?.type !== 'bearclaw:open') return
    const route = openRoute(e.data.url, window.location.origin)
    if (route) navigate(route)
  })
}

/** This device's current push subscription, or null (none, or no push here). */
export async function currentSubscription(): Promise<PushSubscription | null> {
  if (pushSupport(browserPushEnv()) !== 'ok') return null
  const reg = await navigator.serviceWorker.getRegistration()
  return (await reg?.pushManager.getSubscription()) ?? null
}
